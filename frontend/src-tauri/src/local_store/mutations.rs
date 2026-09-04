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

        let local_sequence = next_local_sequence(&transaction, &edit.skill_id)?;
        transaction.execute(
            r#"
            INSERT INTO local_mutations(
                id, skill_id, local_sequence, operation, base_revision, payload_hash, state
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')
            "#,
            params![
                mutation_id,
                edit.skill_id,
                local_sequence,
                edit.operation.as_db_str(),
                edit.base_revision,
                edit.blob_hash,
            ],
        )?;

        transaction.commit()?;

        Ok(LocalMutation {
            id: mutation_id,
            skill_id: edit.skill_id,
            local_sequence: local_sequence.try_into().unwrap_or(u64::MAX),
            operation: edit.operation,
            base_revision: edit.base_revision,
            payload_hash: edit.blob_hash,
            state: MutationState::Pending,
            retry_count: 0,
            next_attempt_at: None,
            last_attempt_at: None,
            server_error_code: None,
            server_error_details: None,
            acknowledged_remote_revision: None,
            acknowledged_remote_id: None,
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
        let local_sequence = next_local_sequence(&transaction, skill_id)?;
        transaction.execute(
            r#"
            INSERT INTO local_mutations(
                id, skill_id, local_sequence, operation, base_revision, payload_hash, state
            ) VALUES (?1, ?2, ?3, 'delete', ?4, ?5, 'pending')
            "#,
            params![mutation_id, skill_id, local_sequence, base_revision, payload_hash],
        )?;
        transaction.commit()?;

        Ok(LocalMutation {
            id: mutation_id,
            skill_id: skill_id.to_owned(),
            local_sequence: local_sequence.try_into().unwrap_or(u64::MAX),
            operation: MutationOperation::Delete,
            base_revision,
            payload_hash,
            state: MutationState::Pending,
            retry_count: 0,
            next_attempt_at: None,
            last_attempt_at: None,
            server_error_code: None,
            server_error_details: None,
            acknowledged_remote_revision: None,
            acknowledged_remote_id: None,
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

    /// Atomically claims only dependency-safe outbox work. A mutation is
    /// dispatchable only when every earlier mutation for the same Skill is ACKed.
    pub fn claim_dispatchable_mutations(
        &self,
        limit: u32,
    ) -> Result<Vec<LocalMutation>, LocalStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidates = read_dispatchable(&transaction, limit)?;
        let mut claimed = Vec::with_capacity(candidates.len());

        for candidate in candidates {
            let changed = transaction.execute(
                r#"
                UPDATE local_mutations
                SET state = 'in_flight',
                    retry_count = retry_count + 1,
                    last_attempt_at = CURRENT_TIMESTAMP,
                    next_attempt_at = NULL,
                    updated_at = CURRENT_TIMESTAMP
                WHERE id = ?1 AND state IN ('pending', 'retryable_error')
                "#,
                [candidate.id.as_str()],
            )?;
            if changed != 1 {
                return Err(LocalStoreError::OutboxClaimLost(candidate.id));
            }
            claimed.push(read_mutation(&transaction, &candidate.id)?);
        }

        transaction.commit()?;
        Ok(claimed)
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
                next_attempt_at = NULL,
                last_error = 'recovered after client restart with unknown remote outcome',
                updated_at = CURRENT_TIMESTAMP
            WHERE state = 'in_flight'
            "#,
            [],
        )?;
        Ok(changed as u64)
    }
}

fn next_local_sequence(
    connection: &rusqlite::Connection,
    skill_id: &str,
) -> Result<i64, LocalStoreError> {
    connection
        .query_row(
            r#"
            SELECT COALESCE(MAX(local_sequence), 0) + 1
            FROM local_mutations
            WHERE skill_id = ?1
            "#,
            [skill_id],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn read_dispatchable(
    connection: &rusqlite::Connection,
    limit: u32,
) -> Result<Vec<LocalMutation>, LocalStoreError> {
    let mut statement = connection.prepare(
        r#"
        SELECT m.id, m.skill_id, m.local_sequence, m.operation, m.base_revision,
               m.payload_hash, m.state, m.retry_count, m.next_attempt_at,
               m.last_attempt_at, m.server_error_code, m.server_error_details,
               m.acknowledged_remote_revision, m.acknowledged_remote_id, m.last_error
        FROM local_mutations m
        WHERE m.state IN ('pending', 'retryable_error')
          AND (m.next_attempt_at IS NULL OR datetime(m.next_attempt_at) <= CURRENT_TIMESTAMP)
          AND NOT EXISTS (
              SELECT 1
              FROM local_mutations earlier
              WHERE earlier.skill_id = m.skill_id
                AND earlier.local_sequence < m.local_sequence
                AND earlier.state != 'acked'
          )
        ORDER BY m.created_at ASC, m.id ASC
        LIMIT ?1
        "#,
    )?;

    let rows = statement.query_map([i64::from(limit)], raw_mutation_from_row)?;
    rows.map(|row| row.map_err(LocalStoreError::from).and_then(RawMutation::into_local))
        .collect()
}

fn read_mutation(
    connection: &rusqlite::Connection,
    mutation_id: &str,
) -> Result<LocalMutation, LocalStoreError> {
    let raw = connection
        .query_row(
            r#"
            SELECT id, skill_id, local_sequence, operation, base_revision,
                   payload_hash, state, retry_count, next_attempt_at, last_attempt_at,
                   server_error_code, server_error_details, acknowledged_remote_revision,
                   acknowledged_remote_id, last_error
            FROM local_mutations
            WHERE id = ?1
            "#,
            [mutation_id],
            raw_mutation_from_row,
        )?;
    raw.into_local()
}

#[derive(Debug)]
struct RawMutation {
    id: String,
    skill_id: String,
    local_sequence: i64,
    operation: String,
    base_revision: Option<i64>,
    payload_hash: String,
    state: String,
    retry_count: u32,
    next_attempt_at: Option<String>,
    last_attempt_at: Option<String>,
    server_error_code: Option<String>,
    server_error_details: Option<String>,
    acknowledged_remote_revision: Option<i64>,
    acknowledged_remote_id: Option<String>,
    last_error: Option<String>,
}

impl RawMutation {
    fn into_local(self) -> Result<LocalMutation, LocalStoreError> {
        let local_sequence = self.local_sequence.try_into().map_err(|_| {
            LocalStoreError::InvalidPersistedState(format!(
                "invalid mutation local_sequence {} for {}",
                self.local_sequence, self.id
            ))
        })?;
        Ok(LocalMutation {
            id: self.id,
            skill_id: self.skill_id,
            local_sequence,
            operation: MutationOperation::from_db_str(&self.operation)?,
            base_revision: self.base_revision,
            payload_hash: self.payload_hash,
            state: MutationState::from_db_str(&self.state)?,
            retry_count: self.retry_count,
            next_attempt_at: self.next_attempt_at,
            last_attempt_at: self.last_attempt_at,
            server_error_code: self.server_error_code,
            server_error_details: self.server_error_details,
            acknowledged_remote_revision: self.acknowledged_remote_revision,
            acknowledged_remote_id: self.acknowledged_remote_id,
            last_error: self.last_error,
        })
    }
}

fn raw_mutation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawMutation> {
    Ok(RawMutation {
        id: row.get(0)?,
        skill_id: row.get(1)?,
        local_sequence: row.get(2)?,
        operation: row.get(3)?,
        base_revision: row.get(4)?,
        payload_hash: row.get(5)?,
        state: row.get(6)?,
        retry_count: row.get(7)?,
        next_attempt_at: row.get(8)?,
        last_attempt_at: row.get(9)?,
        server_error_code: row.get(10)?,
        server_error_details: row.get(11)?,
        acknowledged_remote_revision: row.get(12)?,
        acknowledged_remote_id: row.get(13)?,
        last_error: row.get(14)?,
    })
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
        assert_eq!(mutation.local_sequence, 1);
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
        assert_eq!(claimed[0].retry_count, 1);
        assert!(store.claim_dispatchable_mutations(10).expect("second claim").is_empty());

        assert_eq!(store.recover_in_flight_mutations().expect("recover"), 1);
        let retried = store.claim_dispatchable_mutations(10).expect("retry claim");
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].id, original.id);
        assert_eq!(retried[0].retry_count, 2);
    }

    #[test]
    fn later_mutation_for_same_skill_waits_for_ack() {
        let (_temp, store) = open_temp_store();
        let first = store.commit_skill_edit(sample_edit("skill-1")).expect("first");
        let mut second_edit = sample_edit("skill-1");
        second_edit.operation = MutationOperation::Update;
        let second = store.commit_skill_edit(second_edit).expect("second");

        assert_eq!(first.local_sequence, 1);
        assert_eq!(second.local_sequence, 2);
        let dispatchable = store.list_dispatchable_mutations(10).expect("dispatchable");
        assert_eq!(dispatchable.len(), 1);
        assert_eq!(dispatchable[0].id, first.id);

        {
            let connection = store.lock_connection().expect("connection");
            connection
                .execute(
                    "UPDATE local_mutations SET state = 'acked' WHERE id = ?1",
                    [first.id],
                )
                .expect("ack first");
        }
        let dispatchable = store.list_dispatchable_mutations(10).expect("dispatchable");
        assert_eq!(dispatchable.len(), 1);
        assert_eq!(dispatchable[0].id, second.id);
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
    fn delete_waits_behind_unacked_create() {
        let (_temp, store) = open_temp_store();
        let created = store.commit_skill_edit(sample_edit("skill-1")).expect("commit");
        let deletion = store.queue_skill_delete("skill-1", None).expect("delete");

        assert_eq!(deletion.operation, MutationOperation::Delete);
        assert_eq!(deletion.local_sequence, created.local_sequence + 1);
        assert!(store.get_skill("skill-1").expect("read").is_some());
        let dispatchable = store.list_dispatchable_mutations(10).expect("mutations");
        assert_eq!(dispatchable.len(), 1);
        assert_eq!(dispatchable[0].id, created.id);
    }

    #[test]
    fn workspace_path_must_be_absolute() {
        let (_temp, store) = open_temp_store();
        let mut edit = sample_edit("skill-1");
        edit.workspace_path = PathBuf::from("relative/workspace");
        assert!(store.commit_skill_edit(edit).is_err());
    }
}
