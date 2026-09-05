//! Pull apply transaction (M2.5 desktop, plan §11-§12).
//!
//! One pulled change page is applied in exactly one SQLite transaction:
//! metadata upserts/tombstones, remote revision/package identity, conflict
//! detection against local dirty work, and the cursor commit all happen
//! atomically. The cursor never advances past uncommitted local
//! application, and deletion is always an explicit tombstone event.

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::Deserialize;

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

        for change in &page.changes {
            if change.operation == "delete" {
                apply_tombstone(&transaction, change)?;
                tombstones += 1;
            } else {
                conflicts += apply_upsert(&transaction, change)?;
                upserts += 1;
            }
        }

        // Cursor advance is part of the same transaction: an interrupted
        // apply rolls back the metadata writes AND the cursor.
        super::sync_state::record_pull_cursor_in_transaction(&transaction, &page.next_cursor)?;

        transaction.commit()?;
        Ok(PageApplied {
            upserts,
            tombstones,
            conflicts_detected: conflicts,
        })
    }

    pub fn sync_cursor(&self) -> Result<Option<String>, LocalStoreError> {
        Ok(self.sync_state()?.server_cursor)
    }
}

/// Upserts pulled metadata. Local dirty work never gets silently
/// overwritten: a remote change under a dirty/conflict head marks the
/// resource `conflict` instead.
fn apply_upsert(
    transaction: &rusqlite::Transaction<'_>,
    change: &ChangeItem,
) -> Result<u64, LocalStoreError> {
    let name = metadata_string(change, "name");
    let slug = metadata_string(change, "slug");
    let workspace_path = format!("{}\\{}", std::env::temp_dir().display(), change.resource_id);

    let existing_state: Option<String> = transaction
        .query_row(
            "SELECT sync_state FROM local_skills WHERE id = ?1",
            params![change.resource_id],
            |row| row.get(0),
        )
        .optional()?;

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
            remote_revision = ?6,
            current_blob_hash = COALESCE(?5, local_skills.current_blob_hash),
            name = COALESCE(?2, local_skills.name),
            slug = COALESCE(?3, local_skills.slug),
            sync_state = ?7,
            updated_at = CURRENT_TIMESTAMP
        "#,
        params![
            change.resource_id,
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
    let dirty: bool = transaction
        .query_row(
            "SELECT COUNT(*) FROM local_mutations WHERE skill_id = ?1 AND state != 'acked'",
            params![change.resource_id],
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
            params![change.resource_id, change.resource_revision],
        )?;
        return Ok(());
    }

    transaction.execute(
        "DELETE FROM local_skills WHERE id = ?1",
        params![change.resource_id],
    )?;
    Ok(())
}

fn metadata_string(change: &ChangeItem, key: &str) -> Option<String> {
    change
        .metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::super::SkillSyncState;
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
}
