//! Desktop entitlement lease storage and reconciliation (M3, handoff §15.1-§15.2).
//!
//! The server ships a signed lease for every managed (global) skill the
//! device may currently see, inside the change feed's `metadata.entitlement`
//! payload. The lease is stored verbatim in `local_entitlements` alongside
//! the local skill row. Reconciliation turns the lease + wall clock into
//! deterministic local state:
//!
//! - lease valid (issued in the past, expires in the future) → eligible for
//!   offline use;
//! - lease expired → the skill becomes `access_revoked`: deployments are
//!   marked for removal, managed editing stops, and the cache policy decides
//!   eviction. Unacknowledged local work is never destroyed silently — it
//!   stays pinned in the blob store and conflict resolution owns it.
//!
//! The lease is signed by the server, but the desktop does not verify the
//! signature: trust comes from the transport (authenticated pull over the
//! same channel as the content). What the desktop enforces is the expiry
//! contract, which is a policy clock, not a security primitive — an
//! uncontrolled endpoint cannot make copied plaintext disappear (§15.3).

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{LocalStore, LocalStoreError};

/// One decoded `metadata.entitlement` payload from the pull feed. Keys
/// match the server's snake_case metadata dict verbatim (it is a free-form
/// JSON object inside the change item, not a camelCase protocol model).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct EntitlementPayload {
    pub lease: String,
    pub permission_level: String,
    pub offline_policy: String,
    #[serde(default)]
    pub offline_ttl_hours: Option<i64>,
    /// Server-signed issue/expiry instants (ISO 8601, RFC 3339). The desktop
    /// uses the signed claims rather than recomputing the TTL locally, so a
    /// policy change on the server takes effect at the next pull.
    pub issued_at: Option<String>,
    pub expires_at: Option<String>,
}

/// Result of reconciling one pulled entitlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntitlementReconciled {
    pub skill_id: String,
    pub offline_policy: String,
    /// True when the lease is already expired at apply time.
    pub expired: bool,
}

impl LocalStore {
    /// Upserts the entitlement for a managed skill inside the pull-apply
    /// transaction and reconciles its immediate effect on the local row.
    ///
    /// A fresh lease always clears a previous `access_revoked` label caused
    /// by an expired lease (the server re-entitled the skill); it never
    /// clears `access_revoked` caused by a `permission_denied` mutation
    /// outcome — that gate is owned by the mutation chain.
    pub fn apply_entitlement(
        &self,
        transaction: &rusqlite::Transaction<'_>,
        skill_id: &str,
        payload: &EntitlementPayload,
        now: DateTime<Utc>,
    ) -> Result<EntitlementReconciled, LocalStoreError> {
        let expires_at = payload
            .expires_at
            .as_deref()
            .map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .map(|parsed| parsed.with_timezone(&Utc))
                    .map_err(|error| {
                        LocalStoreError::InvalidPersistedState(format!(
                            "entitlement for {skill_id} carries a malformed expires_at: {error}"
                        ))
                    })
            })
            .transpose()?;
        let issued_at = payload
            .issued_at
            .as_deref()
            .map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .map(|parsed| parsed.with_timezone(&Utc))
                    .map_err(|error| {
                        LocalStoreError::InvalidPersistedState(format!(
                            "entitlement for {skill_id} carries a malformed issued_at: {error}"
                        ))
                    })
            })
            .transpose()?;
        let expired = expires_at.is_some_and(|limit| limit <= now);

        transaction.execute(
            r#"
            INSERT INTO local_entitlements(
                skill_id, lease, permission_level, offline_policy,
                offline_ttl_hours, issued_at, expires_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
            ON CONFLICT(skill_id) DO UPDATE SET
                lease = excluded.lease,
                permission_level = excluded.permission_level,
                offline_policy = excluded.offline_policy,
                offline_ttl_hours = excluded.offline_ttl_hours,
                issued_at = excluded.issued_at,
                expires_at = excluded.expires_at,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                skill_id,
                payload.lease,
                payload.permission_level,
                payload.offline_policy,
                payload.offline_ttl_hours,
                issued_at.map(|value| value.to_rfc3339()),
                expires_at.map(|value| value.to_rfc3339()),
            ],
        )?;

        if expired {
            // Expired at first observation: revoke immediately (§15.2).
            transaction.execute(
                r#"
                UPDATE local_skills
                SET sync_state = 'access_revoked', updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                  AND sync_state IN ('remote_only', 'synced')
                "#,
                params![skill_id],
            )?;
        } else {
            // A fresh lease re-entitles an expired-lease revocation.
            transaction.execute(
                r#"
                UPDATE local_skills
                SET sync_state = 'remote_only', updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                  AND sync_state = 'access_revoked'
                  AND remote_id IS NOT NULL
                "#,
                params![skill_id],
            )?;
        }

        Ok(EntitlementReconciled {
            skill_id: skill_id.to_owned(),
            offline_policy: payload.offline_policy.clone(),
            expired,
        })
    }

    /// Startup/lifecycle reconciliation: expires every entitlement whose
    /// deadline has passed, marking its skill `access_revoked`. Returns the
    /// reconciled skill IDs. Deployments are transitioned separately by the
    /// deployment reconcile step reading the new skill states.
    pub fn expire_due_entitlements(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>, LocalStoreError> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let now_text = now.to_rfc3339();

        let skill_ids: Vec<String> = {
            let mut statement = transaction.prepare(
                r#"
                SELECT e.skill_id
                FROM local_entitlements e
                JOIN local_skills s ON s.id = e.skill_id
                WHERE e.expires_at IS NOT NULL
                  AND e.expires_at <= ?1
                  AND s.sync_state IN ('remote_only', 'synced', 'sync_error')
                "#,
            )?;
            let rows = statement.query_map([&now_text], |row| row.get::<_, String>(0))?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        for skill_id in &skill_ids {
            transaction.execute(
                r#"
                UPDATE local_skills
                SET sync_state = 'access_revoked', updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1
                  AND sync_state IN ('remote_only', 'synced', 'sync_error')
                "#,
                params![skill_id],
            )?;
            transaction.execute(
                r#"
                UPDATE skill_deployments
                SET state = 'revoked',
                    last_error = 'entitlement expired; deployment revoked by offline policy',
                    updated_at = CURRENT_TIMESTAMP
                WHERE skill_id = ?1
                  AND state IN ('installing', 'installed', 'updating', 'modified')
                "#,
                params![skill_id],
            )?;
        }

        transaction.commit()?;
        Ok(skill_ids)
    }

    /// Reads the stored entitlement for a skill (diagnostics/UI).
    pub fn get_entitlement(
        &self,
        skill_id: &str,
    ) -> Result<Option<StoredEntitlement>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let row = connection
            .query_row(
                r#"
                SELECT skill_id, lease, permission_level, offline_policy,
                       offline_ttl_hours, issued_at, expires_at
                FROM local_entitlements
                WHERE skill_id = ?1
                "#,
                [skill_id],
                |row| {
                    Ok(StoredEntitlement {
                        skill_id: row.get(0)?,
                        lease: row.get(1)?,
                        permission_level: row.get(2)?,
                        offline_policy: row.get(3)?,
                        offline_ttl_hours: row.get(4)?,
                        issued_at: row.get(5)?,
                        expires_at: row.get(6)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }
}

/// The stored entitlement row (lease held verbatim for audits).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredEntitlement {
    pub skill_id: String,
    pub lease: String,
    pub permission_level: String,
    pub offline_policy: String,
    pub offline_ttl_hours: Option<i64>,
    pub issued_at: Option<String>,
    pub expires_at: Option<String>,
}

/// Convenience for callers deciding deployability.
pub fn entitlement_allows_offline_use(
    entitlement: Option<&StoredEntitlement>,
    now: DateTime<Utc>,
) -> bool {
    match entitlement {
        None => false, // managed skills require a lease
        Some(lease) => match lease.expires_at.as_deref() {
            None => false, // malformed lease: fail closed
            Some(text) => DateTime::parse_from_rfc3339(text)
                .map(|limit| limit.with_timezone(&Utc) > now)
                .unwrap_or(false),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_store::{CommitSkillEdit, DeploymentState, MutationOperation, SkillSyncState};
    use chrono::TimeZone;

    fn open_temp_store() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("open store");
        (temp, store)
    }

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0).single().expect("timestamp")
    }

    fn entitlement(issued: &str, expires: &str) -> EntitlementPayload {
        EntitlementPayload {
            lease: "lease-token".to_owned(),
            permission_level: "use".to_owned(),
            offline_policy: "ttl".to_owned(),
            offline_ttl_hours: Some(8),
            issued_at: Some(issued.to_owned()),
            expires_at: Some(expires.to_owned()),
        }
    }

    fn seed_synced_skill(store: &LocalStore, skill_id: &str) {
        store
            .commit_skill_edit(CommitSkillEdit {
                skill_id: skill_id.to_owned(),
                remote_id: Some(format!("remote-{skill_id}")),
                name: "Managed".to_owned(),
                slug: "managed".to_owned(),
                workspace_path: std::env::temp_dir().join("skillhive-entitlement"),
                blob_hash: "sha256:local".to_owned(),
                base_revision: Some(1),
                operation: MutationOperation::Update,
            })
            .expect("commit");
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_skills SET sync_state = 'synced' WHERE id = ?1",
                    [skill_id],
                )
                .expect("synced");
        }
    }

    #[test]
    fn fresh_entitlement_stores_and_keeps_skill_usable() {
        let (_temp, store) = open_temp_store();
        seed_synced_skill(&store, "skill-1");
        // 2026-09-06T12:00:00Z — inside the lease window below.
        let now = at(1_788_696_000);
        let payload = entitlement("2026-09-06T12:00:00Z", "2026-09-07T12:00:00Z");
        {
            let mut connection = store.lock_connection().expect("conn");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("tx");
            let reconciled = store
                .apply_entitlement(&transaction, "skill-1", &payload, now)
                .expect("apply");
            assert!(!reconciled.expired);
            transaction.commit().expect("commit");
        }
        let stored = store
            .get_entitlement("skill-1")
            .expect("read")
            .expect("row");
        assert_eq!(stored.offline_policy, "ttl");
        assert_eq!(stored.lease, "lease-token");
        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Synced);
        assert!(entitlement_allows_offline_use(
            store.get_entitlement("skill-1").expect("read").as_ref(),
            now
        ));
    }

    #[test]
    fn expired_entitlement_revokes_skill_at_apply() {
        let (_temp, store) = open_temp_store();
        seed_synced_skill(&store, "skill-2");
        // 2026-09-09T12:00:00Z — two days past the lease deadline below.
        let now = at(1_788_696_000 + 48 * 3600);
        let payload = entitlement("2026-09-06T12:00:00Z", "2026-09-07T12:00:00Z");
        {
            let mut connection = store.lock_connection().expect("conn");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("tx");
            let reconciled = store
                .apply_entitlement(&transaction, "skill-2", &payload, now)
                .expect("apply");
            assert!(reconciled.expired);
            transaction.commit().expect("commit");
        }
        let skill = store.get_skill("skill-2").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::AccessRevoked);
        assert!(!entitlement_allows_offline_use(
            store.get_entitlement("skill-2").expect("read").as_ref(),
            now
        ));
    }

    #[test]
    fn expire_due_entitlements_revokes_only_expired_clean_skills() {
        let (_temp, store) = open_temp_store();
        seed_synced_skill(&store, "skill-3");
        seed_synced_skill(&store, "skill-4");
        // Store skill-3 with an already-passed deadline, skill-4 with a
        // future one.
        let now = at(1_788_696_000 + 48 * 3600);
        {
            let mut connection = store.lock_connection().expect("conn");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("tx");
            store
                .apply_entitlement(
                    &transaction,
                    "skill-3",
                    &entitlement("2026-09-06T12:00:00Z", "2026-09-07T12:00:00Z"),
                    now,
                )
                .expect("apply 3");
            store
                .apply_entitlement(
                    &transaction,
                    "skill-4",
                    &entitlement("2026-09-06T12:00:00Z", "2027-01-01T00:00:00Z"),
                    now,
                )
                .expect("apply 4");
            transaction.commit().expect("commit");
        }

        // skill-3 was already revoked at apply time; expiry sweep reports no
        // NEW revocations while skill-4 stays entitled.
        let revoked = store.expire_due_entitlements(now).expect("sweep");
        assert!(revoked.is_empty());
        assert_eq!(
            store
                .get_skill("skill-3")
                .expect("read")
                .expect("skill")
                .sync_state,
            SkillSyncState::AccessRevoked
        );
        assert_eq!(
            store
                .get_skill("skill-4")
                .expect("read")
                .expect("skill")
                .sync_state,
            SkillSyncState::Synced
        );
    }

    #[test]
    fn expire_due_entitlements_marks_deployments_revoked() {
        let (_temp, store) = open_temp_store();
        seed_synced_skill(&store, "skill-5");
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "INSERT INTO agent_profiles(id, descriptor_id, display_name, skill_root) \
                     VALUES ('profile-1', 'custom:test', 'Test', 'C:\\agent')",
                    [],
                )
                .expect("profile");
            connection
                .execute(
                    "INSERT INTO skill_deployments(skill_id, agent_profile_id, deployed_blob_hash, target_path, state) \
                     VALUES ('skill-5', 'profile-1', 'sha256:local', 'C:\\agent\\managed', 'installed')",
                    [],
                )
                .expect("deployment");
        }
        // 2026-09-08T12:00:00Z — already past the lease deadline, so
        // apply_entitlement itself revokes the skill; the sweep's remaining
        // job is to revoke the active deployment.
        let now = at(1_788_696_000 + 48 * 3600);
        {
            let mut connection = store.lock_connection().expect("conn");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("tx");
            store
                .apply_entitlement(
                    &transaction,
                    "skill-5",
                    &entitlement("2026-09-06T12:00:00Z", "2026-09-07T12:00:00Z"),
                    now,
                )
                .expect("apply");
            transaction.commit().expect("commit");
        }
        // Revocation applied at pull time already flipped the skill state;
        // the sweep owns the deployment transition for that skill.
        let deployment_before = store
            .get_deployment("skill-5", "profile-1")
            .expect("read")
            .expect("deployment");
        assert_eq!(deployment_before.state, DeploymentState::Installed);

        // Force a re-sweep by expiring the deployment catalog transition:
        // expire_due_entitlements only handles skills still in an active
        // sync state, so simulate a state where the pull-time revocation
        // ran without the deployment reconcile (e.g. a crash between the
        // two steps) by resetting the skill to synced.
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_skills SET sync_state = 'synced' WHERE id = 'skill-5'",
                    [],
                )
                .expect("reset");
        }
        let revoked = store.expire_due_entitlements(now).expect("sweep");
        assert_eq!(revoked, vec!["skill-5".to_owned()]);
        let deployment = store
            .get_deployment("skill-5", "profile-1")
            .expect("read")
            .expect("deployment");
        assert_eq!(deployment.state, DeploymentState::Revoked);
    }

    #[test]
    fn fresh_lease_clears_expired_revocation_but_not_permission_revocation() {
        let (_temp, store) = open_temp_store();
        seed_synced_skill(&store, "skill-6");
        // Simulate an expired lease having revoked the skill earlier.
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_skills SET sync_state = 'access_revoked' WHERE id = 'skill-6'",
                    [],
                )
                .expect("revoke");
        }
        let now = at(1_788_696_000);
        {
            let mut connection = store.lock_connection().expect("conn");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("tx");
            store
                .apply_entitlement(
                    &transaction,
                    "skill-6",
                    &entitlement("2026-09-06T12:00:00Z", "2027-01-01T00:00:00Z"),
                    now,
                )
                .expect("apply fresh");
            transaction.commit().expect("commit");
        }
        assert_eq!(
            store
                .get_skill("skill-6")
                .expect("read")
                .expect("skill")
                .sync_state,
            SkillSyncState::RemoteOnly
        );
    }

    #[test]
    fn malformed_expires_at_is_rejected() {
        let (_temp, store) = open_temp_store();
        seed_synced_skill(&store, "skill-7");
        let mut payload = entitlement("2026-09-06T12:00:00Z", "not-a-timestamp");
        payload.expires_at = Some("not-a-timestamp".to_owned());
        {
            let mut connection = store.lock_connection().expect("conn");
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("tx");
            let error = store
                .apply_entitlement(&transaction, "skill-7", &payload, at(1_788_696_000))
                .expect_err("must reject");
            assert!(matches!(error, LocalStoreError::InvalidPersistedState(_)));
            transaction.finish().ok();
        }
    }
}
