//! Conflict state persistence and resolution operations (M2.7, plan §14).
//!
//! When a mutation conflicts, the durable state preserves everything the
//! user needs to choose an outcome (plan §14 conflict state):
//!
//! - the local immutable snapshot (blob store, referenced by
//!   `local_skills.current_blob_hash`);
//! - the local pending mutation row (`local_mutations.state = 'conflict'`,
//!   never deleted by the worker);
//! - the remote head revision (recorded on the mutation row via
//!   `apply_mutation_outcome` / pull conflict paths);
//! - the remote package hash (`local_skills.remote_blob_hash`); the local
//!   snapshot in `current_blob_hash` stays pinned until resolution;
//! - the remote metadata needed for the choice (name/slug on
//!   `local_skills`).
//!
//! The resolution modes here are the initial M2 set — automatic three-way
//! package merge is explicitly out of scope:
//!
//! - [`resolve_keep_local`]: after explicit confirmation, re-queues the
//!   local work as a new update mutation against the latest known remote
//!   revision (the server's receipt-keyed idempotency and base-revision
//!   check decide the rest);
//! - [`resolve_keep_remote`]: explicitly drops the local pending chain and
//!   adopts the remote head as the local state.

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{
    mutations::next_local_sequence, LocalStore, LocalStoreError, MutationOperation, SkillSyncState,
};

/// One resolvable conflict: the local Skill record plus the pending
/// mutation that carries the user's unacknowledged work.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictRecord {
    pub skill_id: String,
    pub local_name: String,
    pub local_slug: String,
    pub local_snapshot_hash: String,
    pub local_base_revision: Option<i64>,
    /// Remote head revision as last observed by pull or the server's
    /// conflict response. `None` means the remote head is unknown.
    pub remote_head_revision: Option<i64>,
    /// Remote package identity, when the conflict response or pull supplied
    /// one. The local snapshot remains separate until resolution.
    pub remote_package_manifest_hash: Option<String>,
    pub mutation_id: String,
    pub mutation_operation: MutationOperation,
}

/// Result of applying a resolution.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionApplied {
    /// The new mutation queued by keep-local, if any.
    pub new_mutation_id: Option<String>,
    pub skill_state: SkillSyncState,
}

impl LocalStore {
    /// Lists every Skill currently in `conflict` with its pending mutation
    /// row (the oldest unacked one — the chain head the user is choosing
    /// about). Powers the UI's conflict picker.
    pub fn list_conflicts(&self) -> Result<Vec<ConflictRecord>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT s.id, s.name, s.slug, s.current_blob_hash,
                   m.base_revision, s.remote_revision, s.remote_blob_hash,
                   m.id, m.operation
            FROM local_skills s
            JOIN local_mutations m
              ON m.skill_id = s.id AND m.state = 'conflict'
            WHERE s.sync_state = 'conflict'
              AND m.local_sequence = (
                  SELECT MIN(m2.local_sequence)
                  FROM local_mutations m2
                  WHERE m2.skill_id = s.id AND m2.state = 'conflict'
              )
            ORDER BY s.id ASC
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })?;

        rows.map(|row| {
            let (
                skill_id,
                local_name,
                local_slug,
                local_snapshot_hash,
                local_base_revision,
                remote_head_revision,
                remote_package_manifest_hash,
                mutation_id,
                operation,
            ) = row?;
            Ok(ConflictRecord {
                skill_id,
                local_name,
                local_slug,
                local_snapshot_hash,
                local_base_revision,
                remote_head_revision,
                remote_package_manifest_hash,
                mutation_id,
                mutation_operation: MutationOperation::from_db_str(&operation)?,
            })
        })
        .collect()
    }

    /// Keep-local resolution: after explicit user confirmation, re-queue
    /// the conflicted snapshot as a new update mutation against the latest
    /// known remote head. The previous conflict mutation is retired as
    /// `acked`-equivalent superseded state — actually marked `acked` with a
    /// diagnostic so the dependency chain unblocks; the new mutation
    /// carries the real content decision. Requires the remote head to be
    /// known; without it the client cannot build a valid update.
    pub fn resolve_keep_local(
        &self,
        skill_id: &str,
        confirmed: bool,
    ) -> Result<ResolutionApplied, LocalStoreError> {
        if !confirmed {
            return Err(LocalStoreError::InvalidInput(
                "keep-local resolution requires explicit user confirmation".to_owned(),
            ));
        }

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let skill = read_skill_for_resolution(&transaction, skill_id)?;
        let remote_head = skill
            .remote_revision
            .ok_or_else(|| {
                LocalStoreError::InvalidInput(format!(
                    "cannot keep local for {skill_id}: remote head revision unknown"
                ))
            })?
            .max(1);

        // Retire the conflicted mutation chain: mark every conflicted
        // mutation for this Skill superseded so the dependency gate opens
        // for the new resolution mutation. Their IDs are never reused.
        supersede_conflict_chain(&transaction, skill_id)?;

        let new_mutation_id = uuid::Uuid::new_v4().to_string();
        let local_sequence = next_local_sequence(&transaction, skill_id).and_then(|value| {
            u64::try_from(value).map_err(|_| {
                LocalStoreError::InvalidPersistedState(format!(
                    "invalid local_sequence {value} for {skill_id}"
                ))
            })
        })?;
        transaction.execute(
            r#"
            INSERT INTO local_mutations(
                id, skill_id, local_sequence, operation, base_revision,
                payload_hash, state
            ) VALUES (?1, ?2, ?3, 'update', ?4, ?5, 'pending')
            "#,
            params![
                new_mutation_id,
                skill_id,
                local_sequence as i64,
                remote_head,
                skill.local_snapshot_hash,
            ],
        )?;

        // The Skill leaves `conflict` into `dirty`: it now has pending
        // work again (the re-queued update), which the normal push cycle
        // dispatches like any other mutation.
        transaction.execute(
            r#"
            UPDATE local_skills
            SET sync_state = 'dirty',
                remote_revision = ?2,
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![skill_id, remote_head],
        )?;

        transaction.commit()?;
        Ok(ResolutionApplied {
            new_mutation_id: Some(new_mutation_id),
            skill_state: SkillSyncState::Dirty,
        })
    }

    /// Keep-remote resolution: explicitly drop the local pending chain and
    /// adopt the remote head as the local state. The local snapshot blobs
    /// remain in the content-addressed store (garbage collection owns
    /// them); the workspace keeps its files until the next pull materializes
    /// the remote content. Requires explicit confirmation.
    pub fn resolve_keep_remote(
        &self,
        skill_id: &str,
        confirmed: bool,
    ) -> Result<ResolutionApplied, LocalStoreError> {
        if !confirmed {
            return Err(LocalStoreError::InvalidInput(
                "keep-remote resolution requires explicit user confirmation".to_owned(),
            ));
        }

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let skill = read_skill_for_resolution(&transaction, skill_id)?;
        let remote_package_manifest_hash = skill
            .remote_blob_hash
            .filter(|hash| !hash.is_empty())
            .ok_or_else(|| {
                LocalStoreError::InvalidInput(format!(
                    "cannot keep remote for {skill_id}: remote package hash unknown; pull the remote head first"
                ))
            })?;

        supersede_conflict_chain(&transaction, skill_id)?;

        transaction.execute(
            r#"
            UPDATE local_skills
            SET current_blob_hash = ?2,
                remote_blob_hash = NULL,
                sync_state = 'remote_only',
                updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
            "#,
            params![skill_id, remote_package_manifest_hash],
        )?;

        transaction.commit()?;
        Ok(ResolutionApplied {
            new_mutation_id: None,
            skill_state: SkillSyncState::RemoteOnly,
        })
    }
}

struct SkillForResolution {
    remote_revision: Option<i64>,
    local_snapshot_hash: String,
    remote_blob_hash: Option<String>,
}

fn read_skill_for_resolution(
    transaction: &rusqlite::Transaction<'_>,
    skill_id: &str,
) -> Result<SkillForResolution, LocalStoreError> {
    let row = transaction
        .query_row(
            r#"
            SELECT remote_revision, current_blob_hash, remote_blob_hash, sync_state
            FROM local_skills
            WHERE id = ?1
            "#,
            params![skill_id],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;

    let (remote_revision, local_snapshot_hash, remote_blob_hash, sync_state) =
        row.ok_or_else(|| LocalStoreError::SkillNotFound(skill_id.to_owned()))?;
    if sync_state != "conflict" {
        return Err(LocalStoreError::InvalidInput(format!(
            "skill {skill_id} is not conflicted (state: {sync_state})"
        )));
    }
    Ok(SkillForResolution {
        remote_revision,
        local_snapshot_hash,
        remote_blob_hash,
    })
}

/// Marks every conflicted mutation of the Skill superseded. `acked` is the
/// only terminal state that unblocks the dependency chain; the superseded
/// chain's content decision lives on in the replacement mutation (keep-
/// local) or in the adopted remote head (keep-remote). `last_error`
/// records the human-readable reason.
fn supersede_conflict_chain(
    transaction: &rusqlite::Transaction<'_>,
    skill_id: &str,
) -> Result<(), LocalStoreError> {
    transaction.execute(
        r#"
        UPDATE local_mutations
        SET state = 'acked',
            next_attempt_at = NULL,
            server_error_code = 'CONFLICT_RESOLVED',
            last_error = 'superseded by an explicit conflict resolution',
            updated_at = CURRENT_TIMESTAMP
        WHERE skill_id = ?1 AND state = 'conflict'
        "#,
        params![skill_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_store::{CommitSkillEdit, MutationOutcome};

    fn open_temp_store() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("open store");
        (temp, store)
    }

    fn commit_and_conflict(store: &LocalStore, skill_id: &str) -> String {
        let mutation = store
            .commit_skill_edit(CommitSkillEdit {
                skill_id: skill_id.to_owned(),
                remote_id: Some("remote-1".to_owned()),
                name: "Conflicted".to_owned(),
                slug: "conflicted".to_owned(),
                workspace_path: std::env::temp_dir().join("skillhive-conflict"),
                blob_hash: "sha256:local".to_owned(),
                base_revision: Some(1),
                operation: MutationOperation::Update,
            })
            .expect("commit edit");

        store
            .apply_mutation_outcome(&MutationOutcome {
                mutation_id: mutation.id.clone(),
                status: "conflict".to_owned(),
                remote_skill_id: Some("remote-1".to_owned()),
                revision: None,
                conflict_head_revision: Some(7),
                conflict_package_manifest_hash: Some("sha256:remote".to_owned()),
                error_code: Some("REVISION_CONFLICT".to_owned()),
                message: Some("server head advanced".to_owned()),
            })
            .expect("apply conflict");

        // Simulate pull having observed the remote head revision.
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_skills SET remote_revision = 7 WHERE id = ?1",
                    params![skill_id],
                )
                .expect("set remote head");
        }
        mutation.id
    }

    #[test]
    fn list_conflicts_surfaces_skill_and_mutation_state() {
        let (_temp, store) = open_temp_store();
        commit_and_conflict(&store, "skill-1");

        let conflicts = store.list_conflicts().expect("list");
        assert_eq!(conflicts.len(), 1);
        let record = &conflicts[0];
        assert_eq!(record.skill_id, "skill-1");
        assert_eq!(record.local_snapshot_hash, "sha256:local");
        assert_eq!(record.local_base_revision, Some(1));
        assert_eq!(record.remote_head_revision, Some(7));
        assert_eq!(
            record.remote_package_manifest_hash.as_deref(),
            Some("sha256:remote")
        );
        assert_eq!(record.mutation_operation, MutationOperation::Update);
    }

    #[test]
    fn keep_local_requires_explicit_confirmation() {
        let (_temp, store) = open_temp_store();
        commit_and_conflict(&store, "skill-1");

        let error = store
            .resolve_keep_local("skill-1", false)
            .expect_err("must refuse");
        assert!(matches!(error, LocalStoreError::InvalidInput(_)));
        // State untouched.
        assert_eq!(
            store
                .get_skill("skill-1")
                .expect("read")
                .expect("skill")
                .sync_state,
            SkillSyncState::Conflict
        );
    }

    #[test]
    fn keep_local_requeues_update_against_remote_head() {
        let (_temp, store) = open_temp_store();
        commit_and_conflict(&store, "skill-1");

        let applied = store.resolve_keep_local("skill-1", true).expect("resolve");
        assert_eq!(applied.skill_state, SkillSyncState::Dirty);

        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Dirty);
        assert_eq!(skill.remote_revision, Some(7));

        // The new update mutation is dispatchable and carries the remote
        // head as base revision.
        let dispatchable = store.list_dispatchable_mutations(10).expect("dispatchable");
        assert_eq!(dispatchable.len(), 1);
        assert_eq!(dispatchable[0].operation, MutationOperation::Update);
        assert_eq!(dispatchable[0].base_revision, Some(7));
        assert_eq!(dispatchable[0].payload_hash, "sha256:local");

        // The superseded chain no longer appears as conflicted.
        assert!(store.list_conflicts().expect("list").is_empty());
    }

    #[test]
    fn keep_local_refuses_unknown_remote_head() {
        let (_temp, store) = open_temp_store();
        commit_and_conflict(&store, "skill-1");
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_skills SET remote_revision = NULL WHERE id = 'skill-1'",
                    [],
                )
                .expect("clear remote head");
        }

        let error = store
            .resolve_keep_local("skill-1", true)
            .expect_err("must refuse");
        assert!(matches!(error, LocalStoreError::InvalidInput(_)));
        assert_eq!(
            store
                .get_skill("skill-1")
                .expect("read")
                .expect("skill")
                .sync_state,
            SkillSyncState::Conflict
        );
    }

    #[test]
    fn keep_remote_drops_local_chain_and_syncs() {
        let (_temp, store) = open_temp_store();
        commit_and_conflict(&store, "skill-1");

        let applied = store.resolve_keep_remote("skill-1", true).expect("resolve");
        assert_eq!(applied.skill_state, SkillSyncState::RemoteOnly);
        assert_eq!(applied.new_mutation_id, None);

        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::RemoteOnly);
        assert_eq!(skill.current_blob_hash, "sha256:remote");
        assert_eq!(skill.remote_blob_hash, None);
        assert!(store.list_dispatchable_mutations(10).expect("d").is_empty());
        assert!(store.list_conflicts().expect("list").is_empty());
    }

    #[test]
    fn keep_remote_refuses_unconfirmed_call() {
        let (_temp, store) = open_temp_store();
        commit_and_conflict(&store, "skill-1");
        let error = store
            .resolve_keep_remote("skill-1", false)
            .expect_err("must refuse");
        assert!(matches!(error, LocalStoreError::InvalidInput(_)));
    }

    #[test]
    fn keep_remote_refuses_unknown_remote_package() {
        let (_temp, store) = open_temp_store();
        commit_and_conflict(&store, "skill-1");
        {
            let connection = store.lock_connection().expect("conn");
            connection
                .execute(
                    "UPDATE local_skills SET remote_blob_hash = NULL WHERE id = 'skill-1'",
                    [],
                )
                .expect("clear remote hash");
        }

        let error = store
            .resolve_keep_remote("skill-1", true)
            .expect_err("must refuse without remote package hash");
        assert!(matches!(error, LocalStoreError::InvalidInput(_)));
        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Conflict);
        assert_eq!(skill.current_blob_hash, "sha256:local");
    }

    #[test]
    fn resolution_refuses_non_conflicted_skill() {
        let (_temp, store) = open_temp_store();
        store
            .commit_skill_edit(CommitSkillEdit {
                skill_id: "skill-2".to_owned(),
                remote_id: None,
                name: "Clean".to_owned(),
                slug: "clean".to_owned(),
                workspace_path: std::env::temp_dir().join("skillhive-clean"),
                blob_hash: "sha256:clean".to_owned(),
                base_revision: None,
                operation: MutationOperation::Create,
            })
            .expect("commit");

        let error = store
            .resolve_keep_local("skill-2", true)
            .expect_err("must refuse");
        assert!(matches!(error, LocalStoreError::InvalidInput(_)));
        let error = store
            .resolve_keep_remote("skill-2", true)
            .expect_err("must refuse");
        assert!(matches!(error, LocalStoreError::InvalidInput(_)));
    }
}
