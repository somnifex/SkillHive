//! Pull apply transaction (M2.5 desktop, plan §11-§12).
//!
//! One pulled change page is applied in exactly one SQLite transaction:
//! metadata upserts/tombstones, remote revision/package identity, conflict
//! detection against local dirty work, and the cursor commit all happen
//! atomically. The cursor never advances past uncommitted local
//! application, and deletion is always an explicit tombstone event.

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::Deserialize;

use super::entitlements::EntitlementPayload;
use super::{LocalStore, LocalStoreError};

/// One decoded `SyncChangeItem` from the pull feed (camelCase on the wire).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeItem {
    pub sequence: i64,
    #[allow(dead_code)]
    pub resource_type: String,
    pub resource_id: String,
    pub resource_revision: i64,
    pub operation: String,
    #[serde(default)]
    pub package_manifest_hash: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// One decoded `SyncChangesResponse` page.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangesPage {
    #[allow(dead_code)]
    pub protocol_version: u32,
    pub changes: Vec<ChangeItem>,
    pub next_cursor: String,
    pub has_more: bool,
}

/// Result of durably applying one page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageApplied {
    pub upserts: usize,
    pub tombstones: usize,
    pub conflicts_detected: u64,
    pub entitlements: usize,
}

impl LocalStore {
    /// Applies one pulled change page transactionally and commits the page
    /// cursor in the same transaction. Returns per-kind counts and how many
    /// resources became locally conflicted.
    ///
    /// Visibility: a page row the server shipped is authoritative; the
    /// desktop applies it unless it would destroy unacknowledged local
    /// work, in which case the resource enters `conflict` and the local
    /// immutable snapshot stays pinned for resolution (plan §10).
    pub fn apply_changes_page(&self, page: &ChangesPage) -> Result<PageApplied, LocalStoreError> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let mut upserts = 0_usize;
        let mut tombstones = 0_usize;
        let mut conflicts = 0_u64;
        let mut entitlements = 0_usize;

        for change in &page.changes {
            if change.operation == "delete" {
                apply_tombstone(&transaction, change)?;
                tombstones += 1;
            } else {
                conflicts += apply_upsert(&transaction, change)?;
                upserts += 1;
            }
            entitlements += apply_change_entitlement(self, &transaction, change)?;
        }

        // Cursor advance is part of the same transaction: an interrupted
        // apply rolls back the metadata writes AND the cursor.
        super::sync_state::record_pull_cursor_in_transaction(&transaction, &page.next_cursor)?;

        transaction.commit()?;
        Ok(PageApplied {
            upserts,
            tombstones,
            conflicts_detected: conflicts,
            entitlements,
        })
    }

    pub fn sync_cursor(&self) -> Result<Option<String>, LocalStoreError> {
        Ok(self.sync_state()?.server_cursor)
    }
}

/// Upserts pulled metadata. Local dirty work never gets silently
/// overwritten: a remote change under a dirty/conflict head marks the
/// resource `conflict` instead.
///
/// Identity: a local row for a pushed-but-created Skill carries a
/// client-generated primary key with the server ID in `remote_id`
/// (UNIQUE). When the server echoes that Skill back through the change
/// feed, `resource_id` IS the server ID, so the upsert must first match
/// on `remote_id` and adopt that row's primary key — inserting a second
/// row keyed by the server ID would collide with the UNIQUE constraint
/// and roll back the whole page. The merge keeps the local key (outbox
/// chain, deployments and workspaces reference it) and refreshes the
/// remote identity columns.
fn apply_upsert(
    transaction: &rusqlite::Transaction<'_>,
    change: &ChangeItem,
) -> Result<u64, LocalStoreError> {
    let name = metadata_string(change, "name");
    let slug = metadata_string(change, "slug");
    let workspace_path = format!("{}\\{}", std::env::temp_dir().display(), change.resource_id);

    let existing: Option<(String, String)> = transaction
        .query_row(
            "SELECT id, sync_state FROM local_skills WHERE id = ?1 OR remote_id = ?1",
            params![change.resource_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let (local_key, existing_state) = existing
        .map(|(id, state)| (Some(id), Some(state)))
        .unwrap_or((None, None));

    let conflict = matches!(
        existing_state.as_deref(),
        Some("dirty") | Some("conflict") | Some("uploading")
    );

    let sync_state = if existing_state.is_none() {
        // Remote-only: metadata known, bytes not cached.
        "remote_only"
    } else if conflict {
        "conflict"
    } else {
        "synced"
    };

    let inserted = transaction.execute(
        r#"
        INSERT INTO local_skills(
            id, remote_id, name, slug, workspace_path, current_blob_hash,
            remote_revision, sync_state, pinned, updated_at
        ) VALUES (?1, ?1, COALESCE(?2, ?1), COALESCE(?3, ?1), ?4, COALESCE(?5, ''),
                  ?6, ?7, 0, CURRENT_TIMESTAMP)
        ON CONFLICT(id) DO UPDATE SET
            remote_id = COALESCE(local_skills.remote_id, excluded.remote_id),
            remote_revision = ?6,
            current_blob_hash = COALESCE(?5, local_skills.current_blob_hash),
            name = COALESCE(?2, local_skills.name),
            slug = COALESCE(?3, local_skills.slug),
            sync_state = ?7,
            updated_at = CURRENT_TIMESTAMP
        "#,
        params![
            local_key.as_deref().unwrap_or(change.resource_id.as_str()),
            name,
            slug,
            workspace_path,
            change.package_manifest_hash,
            change.resource_revision,
            sync_state,
        ],
    )?;

    let _ = inserted;
    Ok(u64::from(conflict))
}

/// Applies an explicit tombstone. The local record stays if unacknowledged
/// local work exists (preserved for export); otherwise the row is removed.
fn apply_tombstone(
    transaction: &rusqlite::Transaction<'_>,
    change: &ChangeItem,
) -> Result<(), LocalStoreError> {
    // The local row may be keyed by the client-generated ID (a pushed
    // create carries the server ID only in `remote_id`), so resolve the
    // key the same way apply_upsert does.
    let local_key: Option<String> = transaction
        .query_row(
            "SELECT id FROM local_skills WHERE id = ?1 OR remote_id = ?1",
            params![change.resource_id],
            |row| row.get(0),
        )
        .optional()?;

    let Some(local_key) = local_key else {
        // Unknown resource: nothing cached, nothing to remove.
        return Ok(());
    };

    let dirty: bool = transaction
        .query_row(
            "SELECT COUNT(*) FROM local_mutations WHERE skill_id = ?1 AND state != 'acked'",
            params![local_key],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)?;

    if dirty {
        transaction.execute(
            r#"
            UPDATE local_skills
            SET remote_revision = ?2, sync_state = 'conflict', updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![local_key, change.resource_revision],
        )?;
        return Ok(());
    }

    transaction.execute("DELETE FROM local_skills WHERE id = ?1", params![local_key])?;
    Ok(())
}

fn metadata_string(change: &ChangeItem, key: &str) -> Option<String> {
    change
        .metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

/// Applies a shipped entitlement lease (M3) inside the same page
/// transaction. Tombstones carry no lease; an upsert's `metadata.entitlement`
/// is optional (personal skills ship none). A malformed lease payload fails
/// the whole page so the cursor never advances past a lease the desktop
/// could not interpret (fail closed).
fn apply_change_entitlement(
    store: &LocalStore,
    transaction: &rusqlite::Transaction<'_>,
    change: &ChangeItem,
) -> Result<usize, LocalStoreError> {
    if change.operation == "delete" {
        return Ok(0);
    }
    let Some(entitlement_json) = change.metadata.get("entitlement") else {
        return Ok(0);
    };
    if entitlement_json.is_null() {
        return Ok(0);
    }
    let payload: EntitlementPayload =
        serde_json::from_value(entitlement_json.clone()).map_err(|error| {
            LocalStoreError::InvalidPersistedState(format!(
                "change {} carries a malformed entitlement: {error}",
                change.resource_id
            ))
        })?;
    // The lease binds the server ID; resolve the local key the same way the
    // upsert did so the entitlement lands on the row the outbox chains to.
    let local_key: Option<String> = transaction
        .query_row(
            "SELECT id FROM local_skills WHERE id = ?1 OR remote_id = ?1",
            params![change.resource_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(local_key) = local_key else {
        // No local row yet: the upsert above always creates one, so this is
        // unreachable in practice; skipping is still safe because the next
        // pull re-ships the lease.
        return Ok(0);
    };
    store.apply_entitlement(transaction, &local_key, &payload, chrono::Utc::now())?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::super::entitlements::entitlement_allows_offline_use;
    use super::super::{CommitSkillEdit, MutationOperation, SkillSyncState};
    use super::*;
    use serde_json::json;

    fn open_temp_store() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("open store");
        (temp, store)
    }

    fn upsert(id: &str, revision: i64) -> ChangeItem {
        ChangeItem {
            sequence: revision,
            resource_type: "skill".to_owned(),
            resource_id: id.to_owned(),
            resource_revision: revision,
            operation: "upsert".to_owned(),
            package_manifest_hash: Some("sha256:abc".to_owned()),
            metadata: json!({"name": "Pulled Skill", "slug": "pulled-skill"}),
        }
    }

    fn tombstone(id: &str, revision: i64) -> ChangeItem {
        ChangeItem {
            sequence: revision,
            resource_type: "skill".to_owned(),
            resource_id: id.to_owned(),
            resource_revision: revision,
            operation: "delete".to_owned(),
            package_manifest_hash: None,
            metadata: json!({}),
        }
    }

    fn page(changes: Vec<ChangeItem>, cursor: &str) -> ChangesPage {
        ChangesPage {
            protocol_version: 1,
            changes,
            next_cursor: cursor.to_owned(),
            has_more: false,
        }
    }

    #[test]
    fn pulled_upsert_creates_remote_only_record_and_commits_cursor() {
        let (_temp, store) = open_temp_store();
        let applied = store
            .apply_changes_page(&page(vec![upsert("remote-1", 1)], "v1.AAAAAQ"))
            .expect("apply");

        assert_eq!(applied.upserts, 1);
        let skill = store.get_skill("remote-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::RemoteOnly);
        assert_eq!(skill.remote_revision, Some(1));
        assert_eq!(skill.remote_id.as_deref(), Some("remote-1"));
        assert_eq!(skill.name, "Pulled Skill");
        assert_eq!(
            store.sync_state().expect("state").server_cursor.as_deref(),
            Some("v1.AAAAAQ")
        );
    }

    #[test]
    fn interrupted_apply_rolls_back_metadata_and_cursor() {
        let (_temp, store) = open_temp_store();
        // A cursor validation failure mid-transaction must roll back everything.
        let bad_page = ChangesPage {
            protocol_version: 1,
            changes: vec![upsert("remote-2", 1)],
            next_cursor: "   ".to_owned(),
            has_more: false,
        };
        assert!(store.apply_changes_page(&bad_page).is_err());
        assert!(store.get_skill("remote-2").expect("read").is_none());
    }

    #[test]
    fn tombstone_removes_clean_record() {
        let (_temp, store) = open_temp_store();
        store
            .apply_changes_page(&page(vec![upsert("remote-3", 1)], "v1.AAAAAQ"))
            .expect("upsert");
        store
            .apply_changes_page(&page(vec![tombstone("remote-3", 2)], "v1.AAAAFA"))
            .expect("delete");
        assert!(store.get_skill("remote-3").expect("read").is_none());
    }

    #[test]
    fn tombstone_with_pending_mutation_preserves_work_as_conflict() {
        let (_temp, store) = open_temp_store();
        store
            .apply_changes_page(&page(vec![upsert("remote-4", 1)], "v1.AAAAAQ"))
            .expect("upsert");
        // Simulate a pending local mutation.
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "INSERT INTO local_mutations(id, skill_id, local_sequence, operation, payload_hash, state) \
                     VALUES ('m-1', 'remote-4', 1, 'update', 'sha256:abc', 'pending')",
                    [],
                )
                .expect("insert mutation");
        }

        store
            .apply_changes_page(&page(vec![tombstone("remote-4", 2)], "v1.AAAAFA"))
            .expect("delete");
        let skill = store.get_skill("remote-4").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Conflict);
    }

    #[test]
    fn remote_change_under_dirty_head_conflicts_instead_of_overwriting() {
        let (_temp, store) = open_temp_store();
        store
            .apply_changes_page(&page(vec![upsert("remote-5", 1)], "v1.AAAAAQ"))
            .expect("upsert");
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_skills SET sync_state = 'dirty' WHERE id = 'remote-5'",
                    [],
                )
                .expect("dirty");
        }

        let applied = store
            .apply_changes_page(&page(vec![upsert("remote-5", 5)], "v1.AAAABQ"))
            .expect("apply");
        assert_eq!(applied.conflicts_detected, 1);
        let skill = store.get_skill("remote-5").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Conflict);
        assert_eq!(skill.remote_revision, Some(5));
    }

    #[test]
    fn pulled_upsert_merges_into_locally_keyed_row_by_remote_id() {
        // The push path keys the local row by a client-generated ID and
        // stores the server ID only in `remote_id` (UNIQUE). When the feed
        // echoes the same resource, the upsert must merge into that row
        // instead of inserting a second one keyed by the server ID.
        let (_temp, store) = open_temp_store();
        let mutation = store
            .commit_skill_edit(CommitSkillEdit {
                skill_id: "local-key-1".to_owned(),
                remote_id: None,
                name: "Local First".to_owned(),
                slug: "local-first".to_owned(),
                workspace_path: std::env::temp_dir().join("skillhive-merge"),
                blob_hash: "sha256:local".to_owned(),
                base_revision: None,
                operation: MutationOperation::Create,
            })
            .expect("commit");
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_mutations SET state = 'acked' WHERE id = ?1",
                    [mutation.id.as_str()],
                )
                .expect("ack");
            connection
                .execute(
                    "UPDATE local_skills SET remote_id = 'srv-9', remote_revision = 1, \
                     sync_state = 'synced' WHERE id = 'local-key-1'",
                    [],
                )
                .expect("attach remote id");
        }

        let applied = store
            .apply_changes_page(&page(vec![upsert("srv-9", 2)], "v1.AAAAQg"))
            .expect("apply");

        assert_eq!(applied.upserts, 1);
        // Same local key, refreshed remote identity; no second row.
        let skill = store
            .get_skill("local-key-1")
            .expect("read")
            .expect("skill");
        assert_eq!(skill.remote_id.as_deref(), Some("srv-9"));
        assert_eq!(skill.remote_revision, Some(2));
        assert_eq!(skill.sync_state, SkillSyncState::Synced);
        assert!(store.get_skill("srv-9").expect("read").is_none());
    }

    #[test]
    fn pulled_tombstone_removes_locally_keyed_row_by_remote_id() {
        let (_temp, store) = open_temp_store();
        let mutation = store
            .commit_skill_edit(CommitSkillEdit {
                skill_id: "local-key-2".to_owned(),
                remote_id: None,
                name: "Local Second".to_owned(),
                slug: "local-second".to_owned(),
                workspace_path: std::env::temp_dir().join("skillhive-tombstone"),
                blob_hash: "sha256:local2".to_owned(),
                base_revision: None,
                operation: MutationOperation::Create,
            })
            .expect("commit");
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_mutations SET state = 'acked' WHERE id = ?1",
                    [mutation.id.as_str()],
                )
                .expect("ack");
            connection
                .execute(
                    "UPDATE local_skills SET remote_id = 'srv-10', remote_revision = 1, \
                     sync_state = 'synced' WHERE id = 'local-key-2'",
                    [],
                )
                .expect("attach remote id");
        }

        store
            .apply_changes_page(&page(vec![tombstone("srv-10", 2)], "v1.AAAAQg"))
            .expect("delete");
        assert!(store.get_skill("local-key-2").expect("read").is_none());
    }

    fn entitlement_metadata(issued: &str, expires: &str) -> serde_json::Value {
        json!({
            "name": "Managed Pull",
            "slug": "managed-pull",
            "entitlement": {
                "lease": "lease-token",
                "permission_level": "use",
                "offline_policy": "ttl",
                "offline_ttl_hours": 8,
                "issued_at": issued,
                "expires_at": expires,
            },
        })
    }

    #[test]
    fn pulled_entitlement_stores_alongside_content() {
        let (_temp, store) = open_temp_store();
        let mut item = upsert("remote-ent-1", 1);
        item.metadata = entitlement_metadata("2026-09-06T12:00:00Z", "2027-01-01T00:00:00Z");

        let applied = store
            .apply_changes_page(&page(vec![item], "v1.AAAAAQ"))
            .expect("apply");

        assert_eq!(applied.entitlements, 1);
        assert_eq!(applied.upserts, 1);
        let stored = store
            .get_entitlement("remote-ent-1")
            .expect("read")
            .expect("entitlement row");
        assert_eq!(stored.offline_policy, "ttl");
        assert_eq!(stored.offline_ttl_hours, Some(8));
        // Fresh lease: the skill stays usable.
        let skill = store
            .get_skill("remote-ent-1")
            .expect("read")
            .expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::RemoteOnly);
    }

    #[test]
    fn pulled_expired_entitlement_revokes_at_apply() {
        let (_temp, store) = open_temp_store();
        let mut item = upsert("remote-ent-2", 1);
        // A deadline far in the past: the reconciler stamps `now` from the
        // real wall clock, so the expired lease must revoke the fresh upsert
        // immediately at apply time.
        item.metadata = entitlement_metadata("2025-01-01T00:00:00Z", "2025-01-02T00:00:00Z");
        let applied = {
            let mut connection = store.lock_connection().expect("conn");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("tx");
            let mut applied = PageApplied {
                upserts: 0,
                tombstones: 0,
                conflicts_detected: 0,
                entitlements: 0,
            };
            for change in &page(vec![item], "v1.AAAAAQ").changes {
                if change.operation == "delete" {
                    apply_tombstone(&transaction, change).expect("tombstone");
                } else {
                    applied.upserts += apply_upsert(&transaction, change).expect("upsert") as usize;
                    applied.entitlements +=
                        apply_change_entitlement(&store, &transaction, change).expect("ent");
                }
            }
            transaction.commit().expect("commit");
            applied
        };

        assert_eq!(applied.entitlements, 1);
        let skill = store
            .get_skill("remote-ent-2")
            .expect("read")
            .expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::AccessRevoked);
        assert!(!entitlement_allows_offline_use(
            store
                .get_entitlement("remote-ent-2")
                .expect("read")
                .as_ref(),
            at(1_788_696_000)
        ));
    }

    #[test]
    fn malformed_entitlement_fails_whole_page() {
        let (_temp, store) = open_temp_store();
        let mut item = upsert("remote-ent-3", 1);
        item.metadata = json!({
            "name": "Broken Lease",
            "slug": "broken-lease",
            "entitlement": {"lease": "lease-token"},
        });

        // The malformed lease must fail the page so the cursor never advances
        // past a lease the desktop could not interpret.
        assert!(store
            .apply_changes_page(&page(vec![item], "v1.AAAAAQ"))
            .is_err());
        // Nothing from the page landed, including the metadata upsert.
        assert!(store.get_skill("remote-ent-3").expect("read").is_none());
        assert!(store
            .get_entitlement("remote-ent-3")
            .expect("read")
            .is_none());
    }

    fn at(secs: i64) -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc.timestamp_opt(secs, 0).single().expect("ts")
    }
}
