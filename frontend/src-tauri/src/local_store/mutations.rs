use rusqlite::{params, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

use super::{
    path_to_string, validate_non_empty, CommitSkillEdit, LocalMutation, LocalStore, LocalStoreError,
    MutationOperation, MutationState,
};

impl LocalStore {
    /// Commits a user-visible local edit and its durable sync mutation in the
    /// same SQLite transaction. Callers must not report "saved" before this
    /// method succeeds.
    pub fn commit_skill_edit(&self, edit: CommitSkillEdit) -> Result<LocalMutation, LocalStoreError> {
        validate_edit(&edit)?;
        if edit.operation == MutationOperation::Delete {
            return Err(LocalStoreError::InvalidInput(
                "delete mutations must use queue_skill_delete".to_owned(),
            ));
        }

        let mutation_id = Uuid::new_v4().to_string();
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        transaction.execute(
            r#"
            INSERT INTO local_skills(
                id, remote_id, name, slug, workspace_path, current_blob_hash,
                remote_revision, sync_state, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'dirty', CURRENT_TIMESTAMP)
            ON CONFLICT(id) DO UPDATE SET
                remote_id = COALESCE(excluded.remote_id, local_skills.remote_id),
                name = excluded.name,
                slug = excluded.slug,
                workspace_path = excluded.workspace_path,
                current_blob_hash = excluded.current_blob_hash,
                remote_revision = COALESCE(excluded.remote_revision, local_skills.remote_revision),
                sync_state = 'dirty',
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                edit.skill_id,
                edit.remote_id,
                edit.name,
                edit.slug,
                path_to_string(&edit.workspace_path)?,
                edit.blob_hash,
                edit.base_revision,
            ],
        )?;

        transaction.execute(
            r#"
            INSERT INTO local_mutations(
                id, skill_id, operation, base_revision, payload_hash, state
            ) VALUES (?1, ?2, ?3, ?4, ?5, 'pending')
            "#,
            params![
                mutation_id,
                edit.skill_id,
                edit.operation.as_db_str(),
                edit.base_revision,
                edit.blob_hash,
            ],
        )?;

        transaction.commit()?;

        Ok(LocalMutation {
            id: mutation_id,
            skill_id: edit.skill_id,
            operation: edit.operation,
            base_revision: edit.base_revision,
            payload_hash: edit.blob_hash,
            state: MutationState::Pending,
            retry_count: 0,
            last_error: None,
        })
    }

    /// Queues deletion without removing the local record. The row remains until
    /// the authoritative server acknowledges the delete mutation.
    pub fn queue_skill_delete(
        &self,
        skill_id: &str,
        base_revision: Option<i64>,
    ) -> Result<LocalMutation, LocalStoreError> {
        validate_non_empty("skill_id", skill_id)?;
        if base_revision.is_some_and(|revision| revision < 0) {
            return Err(LocalStoreError::InvalidInput(
                "base_revision must not be negative".to_owned(),
            ));
        }

        let mutation_id = Uuid::new_v4().to_string();
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let payload_hash: Option<String> = transaction
            .query_row(
                "SELECT current_blob_hash FROM local_skills WHERE id = ?1",
                [skill_id],
                |row| row.get(0),
            )
            .optional()?;
        let payload_hash =
            payload_hash.ok_or_else(|| LocalStoreError::SkillNotFound(skill_id.to_owned()))?;

        transaction.execute(
            "UPDATE local_skills SET sync_state = 'dirty', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            [skill_id],
        )?;
        transaction.execute(
            r#"
            INSERT INTO local_mutations(
                id, skill_id, operation, base_revision, payload_hash, state
            ) VALUES (?1, ?2, 'delete', ?3, ?4, 'pending')
            "#,
            params![mutation_id, skill_id, base_revision, payload_hash],
        )?;
        transaction.commit()?;

        Ok(LocalMutation {
            id: mutation_id,
            skill_id: skill_id.to_owned(),
            operation: MutationOperation::Delete,
            base_revision,
            payload_hash,
            state: MutationState::Pending,
            retry_count: 0,
            last_error: None,
        })
    }

    pub fn list_dispatchable_mutations(
        &self,
        limit: u32,
    ) -> Result<Vec<LocalMutation>, LocalStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let connection = self.lock_connection()?;
        read_dispatchable(&connection, limit)
    }

    /// Atomically claims pending work for the sync worker. A second worker using
    /// the same LocalStore cannot claim the same mutation because selection and
    /// state transition occur under one IMMEDIATE transaction.
    pub fn claim_dispatchable_mutations(
        &self,
        limit: u32,
    ) -> Result<Vec<LocalMutation>, LocalStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut mutations = read_dispatchable(&transaction, limit)?;

        for mutation in &mut mutations {
            let changed = transaction.execute(
                r#"
                UPDATE local_mutations
                SET state = 'in_flight', updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1 AND state IN ('pending', 'retryable_error')
                "#,
                [mutation.id.as_str()],
            )?;
            if changed != 1 {
                return Err(LocalStoreError::OutboxClaimLost(mutation.id.clone()));
            }
            mutation.state = MutationState::InFlight;
        }

        transaction.commit()?;
        Ok(mutations)
    }

    /// On process restart an in-flight request has an unknown remote outcome.
    /// Requeue the exact same mutation id; the server sync endpoint must be
    /// idempotent on that id, so retrying cannot duplicate the mutation.
    pub fn recover_in_flight_mutations(&self) -> Result<u64, LocalStoreError> {
        let connection = self.lock_connection()?;
        let changed = connection.execute(
            r#"
            UPDATE local_mutations
            SET state = 'retryable_error',
                last_error = 'recovered after client restart with unknown remote outcome',
                updated_at = CURRENT_TIMESTAMP
            WHERE state = 'in_flight'
            "#,
            [],
        )?;
        Ok(changed as u64)
    }
}

fn read_dispatchable(
    connection: &rusqlite::Connection,
    limit: u32,
) -> Result<Vec<LocalMutation>, LocalStoreError> {
    let mut statement = connection.prepare(
        r#"
        SELECT id, skill_id, operation, base_revision, payload_hash, state,
               retry_count, last_error
        FROM local_mutations
        WHERE state IN ('pending', 'retryable_error')
        ORDER BY created_at ASC, id ASC
        LIMIT ?1
        "#,
    )?;

    let rows = statement.query_map([i64::from(limit)], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<i64>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, u32>(6)?,
            row.get::<_, Option<String>>(7)?,
        ))
    })?;

    rows.map(|row| {
        let row = row?;
        Ok(LocalMutation {
            id: row.0,
            skill_id: row.1,
            operation: MutationOperation::from_db_str(&row.2)?,
            base_revision: row.3,
            payload_hash: row.4,
            state: MutationState::from_db_str(&row.5)?,
            retry_count: row.6,
            last_error: row.7,
        })
    })
    .collect()
}

fn validate_edit(edit: &CommitSkillEdit) -> Result<(), LocalStoreError> {
    for (field, value) in [
        ("skill_id", edit.skill_id.as_str()),
        ("name", edit.name.as_str()),
        ("slug", edit.slug.as_str()),
        ("blob_hash", edit.blob_hash.as_str()),
    ] {
        validate_non_empty(field, value)?;
    }
    if !edit.workspace_path.is_absolute() {
        return Err(LocalStoreError::InvalidInput(
            "workspace_path must be absolute".to_owned(),
        ));
    }
    path_to_string(&edit.workspace_path)?;
    if edit.base_revision.is_some_and(|revision| revision < 0) {
        return Err(LocalStoreError::InvalidInput(
            "base_revision must not be negative".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn open_temp_store() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("open store");
        (temp, store)
    }

    fn sample_edit(skill_id: &str) -> CommitSkillEdit {
        CommitSkillEdit {
            skill_id: skill_id.to_owned(),
            remote_id: None,
            name: "Code Review".to_owned(),
            slug: "code-review".to_owned(),
            workspace_path: std::env::temp_dir().join("skillhive").join("code-review"),
            blob_hash: "sha256:abc123".to_owned(),
            base_revision: None,
            operation: MutationOperation::Create,
        }
    }

    #[test]
    fn local_edit_and_outbox_entry_commit_together() {
        let (_temp, store) = open_temp_store();
        let mutation = store.commit_skill_edit(sample_edit("skill-1")).expect("commit");

        assert_eq!(mutation.state, MutationState::Pending);
        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, super::super::SkillSyncState::Dirty);
        assert_eq!(store.list_dispatchable_mutations(10).expect("mutations").len(), 1);
    }

    #[test]
    fn pending_mutations_survive_reopen() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("skillhive.db");
        {
            let store = LocalStore::open(&path).expect("open store");
            store.commit_skill_edit(sample_edit("skill-1")).expect("commit");
        }

        let reopened = LocalStore::open(&path).expect("reopen store");
        let pending = reopened.list_dispatchable_mutations(10).expect("mutations");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].skill_id, "skill-1");
    }

    #[test]
    fn claim_is_atomic_and_restart_requeues_in_flight_id() {
        let (_temp, store) = open_temp_store();
        let original = store.commit_skill_edit(sample_edit("skill-1")).expect("commit");

        let claimed = store.claim_dispatchable_mutations(10).expect("claim");
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, original.id);
        assert_eq!(claimed[0].state, MutationState::InFlight);
        assert!(store.claim_dispatchable_mutations(10).expect("second claim").is_empty());

        assert_eq!(store.recover_in_flight_mutations().expect("recover"), 1);
        let retried = store.claim_dispatchable_mutations(10).expect("retry claim");
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].id, original.id);
    }

    #[test]
    fn invalid_edit_never_creates_skill_or_mutation() {
        let (_temp, store) = open_temp_store();
        let mut edit = sample_edit("skill-1");
        edit.blob_hash.clear();

        assert!(store.commit_skill_edit(edit).is_err());
        assert!(store.get_skill("skill-1").expect("read").is_none());
        assert!(store.list_dispatchable_mutations(10).expect("mutations").is_empty());
    }

    #[test]
    fn delete_is_queued_without_dropping_local_record() {
        let (_temp, store) = open_temp_store();
        store.commit_skill_edit(sample_edit("skill-1")).expect("commit");
        let deletion = store.queue_skill_delete("skill-1", None).expect("delete");

        assert_eq!(deletion.operation, MutationOperation::Delete);
        assert!(store.get_skill("skill-1").expect("read").is_some());
        assert_eq!(store.list_dispatchable_mutations(10).expect("mutations").len(), 2);
    }

    #[test]
    fn workspace_path_must_be_absolute() {
        let (_temp, store) = open_temp_store();
        let mut edit = sample_edit("skill-1");
        edit.workspace_path = PathBuf::from("relative/workspace");
        assert!(store.commit_skill_edit(edit).is_err());
    }
}
