//! Durable mutation outcome application (M2.4 desktop).
//!
//! Implements the local ACK transaction from the sync plan §13 and the
//! error classification from §15. Every outcome is a single SQLite
//! transaction so a worker crash between HTTP and local apply leaves
//! recoverable state:
//!
//! - `acked`   → mutation ACKed, remote ID/revision recorded; the local
//!   Skill flips to `synced` only when no later unacked mutation remains;
//! - `conflict` → mutation `conflict`, Skill `conflict`, remote head
//!   revision preserved in the mutation row for resolution;
//! - `permission_denied` → mutation `permission_denied`, Skill
//!   `access_revoked`, local snapshot preserved;
//! - `validation_error` → mutation `permanent_error` (no retry storm);
//! - transport failure → `retryable_error` with persisted
//!   `next_attempt_at` (bounded exponential backoff).

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::Deserialize;

use super::{LocalStore, LocalStoreError, MutationState, SkillSyncState};

const BACKOFF_BASE_SECS: u64 = 2;
const BACKOFF_MAX_SECONDS: u64 = 64;
const BACKOFF_MAX_EXPONENT: u32 = 6;

/// A definitive protocol outcome for one claimed mutation, decoded from
/// `SyncMutationResponse` (or produced by the worker for transport errors
/// via [`MutationOutcome::transport_error`]).
#[derive(Debug, Clone, Deserialize)]
pub struct MutationOutcome {
    pub mutation_id: String,
    pub status: String,
    pub remote_skill_id: Option<String>,
    pub revision: Option<i64>,
    pub conflict_head_revision: Option<i64>,
    pub error_code: Option<String>,
    pub message: Option<String>,
}

impl MutationOutcome {
    pub fn transport_error(mutation_id: &str, error_code: &str, message: &str) -> Self {
        Self {
            mutation_id: mutation_id.to_owned(),
            status: "transport_error".to_owned(),
            remote_skill_id: None,
            revision: None,
            conflict_head_revision: None,
            error_code: Some(error_code.to_owned()),
            message: Some(message.to_owned()),
        }
    }
}

/// Result of durably applying one outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeApplied {
    pub mutation_state: MutationState,
    pub skill_state: Option<SkillSyncState>,
}

impl LocalStore {
    /// Transactionally persists one protocol outcome and its local state
    /// transition (plan §13 local ACK transaction).
    ///
    /// A worker crash between HTTP response and this call is safe: the
    /// mutation stays `in_flight` and restart recovery requeues the same
    /// mutation ID, which the server replays from its receipt.
    pub fn apply_mutation_outcome(
        &self,
        outcome: &MutationOutcome,
    ) -> Result<OutcomeApplied, LocalStoreError> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let raw = mutation_row(&transaction, &outcome.mutation_id)?
            .ok_or_else(|| LocalStoreError::OutboxClaimLost(outcome.mutation_id.clone()))?;
        if raw.state == "acked" {
            // Idempotent replay of an already-applied outcome.
            return Ok(OutcomeApplied {
                mutation_state: MutationState::Acked,
                skill_state: read_skill_state(&transaction, &raw.skill_id)?,
            });
        }

        match outcome.status.as_str() {
            "acked" => {
                let revision = outcome.revision.ok_or_else(|| {
                    LocalStoreError::InvalidPersistedState(format!(
                        "acked outcome for {} has no revision",
                        outcome.mutation_id
                    ))
                })?;
                apply_acked(&transaction, &raw, outcome, revision)?;
            }
            "conflict" => {
                apply_definitive_error(&transaction, &raw, outcome, "conflict", "conflict")?;
            }
            "permission_denied" => {
                apply_definitive_error(
                    &transaction,
                    &raw,
                    outcome,
                    "permission_denied",
                    "access_revoked",
                )?;
            }
            "validation_error" => {
                apply_definitive_error(
                    &transaction,
                    &raw,
                    outcome,
                    "permanent_error",
                    "sync_error",
                )?;
            }
            "transport_error" => {
                let seconds = backoff_seconds(raw.retry_count);
                transaction.execute(
                    r#"
                    UPDATE local_mutations
                    SET state = 'retryable_error',
                        retry_count = retry_count + 1,
                        next_attempt_at = datetime(CURRENT_TIMESTAMP, '+' || ?2 || ' seconds'),
                        server_error_code = ?3,
                        server_error_details = ?4,
                        last_error = ?4,
                        updated_at = CURRENT_TIMESTAMP
                    WHERE id = ?1
                    "#,
                    params![
                        outcome.mutation_id,
                        i64::try_from(seconds).unwrap_or(i64::MAX),
                        outcome.error_code,
                        outcome.message
                    ],
                )?;
            }
            other => {
                return Err(LocalStoreError::InvalidPersistedState(format!(
                    "unknown mutation outcome status '{other}' for {}",
                    outcome.mutation_id
                )));
            }
        }

        let skill_state = read_skill_state(&transaction, &raw.skill_id)?;
        transaction.commit()?;

        Ok(OutcomeApplied {
            mutation_state: MutationState::from_db_str(&mutation_state_str(&outcome.status))?,
            skill_state,
        })
    }
}

struct RawMutation {
    skill_id: String,
    state: String,
    retry_count: u32,
}

fn mutation_row(
    transaction: &rusqlite::Transaction<'_>,
    mutation_id: &str,
) -> Result<Option<RawMutation>, LocalStoreError> {
    let row = transaction
        .query_row(
            "SELECT skill_id, state, retry_count FROM local_mutations WHERE id = ?1",
            params![mutation_id],
            |row| {
                let retry_count: i64 = row.get(2)?;
                let retry_count = u32::try_from(retry_count).map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        2,
                        "retry_count".to_owned(),
                        rusqlite::types::Type::Integer,
                    )
                })?;
                Ok(RawMutation {
                    skill_id: row.get(0)?,
                    state: row.get(1)?,
                    retry_count,
                })
            },
        )
        .optional()?;
    Ok(row)
}

fn apply_acked(
    transaction: &rusqlite::Transaction<'_>,
    raw: &RawMutation,
    outcome: &MutationOutcome,
    revision: i64,
) -> Result<(), LocalStoreError> {
    transaction.execute(
        r#"
        UPDATE local_mutations
        SET state = 'acked',
            acknowledged_remote_revision = ?2,
            acknowledged_remote_id = COALESCE(?3, acknowledged_remote_id),
            next_attempt_at = NULL,
            server_error_code = NULL,
            server_error_details = NULL,
            last_error = NULL,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?1
        "#,
        params![outcome.mutation_id, revision, outcome.remote_skill_id],
    )?;

    // Map the create mutation's server resource ID and refresh the revision.
    transaction.execute(
        r#"
        UPDATE local_skills
        SET remote_id = COALESCE(?2, remote_id),
            remote_revision = ?3,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?1
        "#,
        params![raw.skill_id, outcome.remote_skill_id, revision],
    )?;

    // The Skill is fully synchronized only when no later mutation for it is
    // still unacked; otherwise its head state remains governed by the chain.
    transaction.execute(
        r#"
        UPDATE local_skills
        SET sync_state = 'synced', updated_at = CURRENT_TIMESTAMP
        WHERE id = ?1
          AND sync_state IN ('dirty', 'uploading', 'sync_error')
          AND NOT EXISTS (
              SELECT 1 FROM local_mutations later
              WHERE later.skill_id = local_skills.id
                AND later.state != 'acked'
          )
        "#,
        params![raw.skill_id],
    )?;
    Ok(())
}

fn apply_definitive_error(
    transaction: &rusqlite::Transaction<'_>,
    raw: &RawMutation,
    outcome: &MutationOutcome,
    mutation_state: &str,
    skill_state: &str,
) -> Result<(), LocalStoreError> {
    transaction.execute(
        r#"
        UPDATE local_mutations
        SET state = ?2,
            next_attempt_at = NULL,
            server_error_code = ?3,
            server_error_details = ?4,
            last_error = ?4,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?1
        "#,
        params![
            outcome.mutation_id,
            mutation_state,
            outcome.error_code,
            outcome.message
        ],
    )?;

    // Preserve the local immutable snapshot; only the sync state changes so
    // the UI can surface the outcome without losing content.
    transaction.execute(
        "UPDATE local_skills SET sync_state = ?2, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
        params![raw.skill_id, skill_state],
    )?;
    Ok(())
}

/// Bounded exponential backoff: 1, 2, 4, 8, 16, 32, 64 seconds, capped at
/// 300 seconds. Persisted via `next_attempt_at` so the schedule survives
/// restart; a deterministic delay is sufficient for M2's single serialized
/// worker (jitter can be layered in M2.6 without changing this contract).
fn backoff_seconds(retry_count: u32) -> u64 {
    let exponent = retry_count.min(BACKOFF_MAX_EXPONENT);
    BACKOFF_BASE_SECS
        .saturating_pow(exponent)
        .min(BACKOFF_MAX_SECONDS)
}

fn mutation_state_str(status: &str) -> String {
    match status {
        "acked" => "acked".to_owned(),
        "conflict" => "conflict".to_owned(),
        "permission_denied" => "permission_denied".to_owned(),
        "validation_error" => "permanent_error".to_owned(),
        _ => "retryable_error".to_owned(),
    }
}

fn read_skill_state(
    transaction: &rusqlite::Transaction<'_>,
    skill_id: &str,
) -> Result<Option<SkillSyncState>, LocalStoreError> {
    let state: Option<String> = transaction
        .query_row(
            "SELECT sync_state FROM local_skills WHERE id = ?1",
            params![skill_id],
            |row| row.get(0),
        )
        .optional()?;
    state
        .map(|value| SkillSyncState::from_db_str(&value))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_store::{CommitSkillEdit, MutationOperation};

    fn open_temp_store() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("open store");
        (temp, store)
    }

    fn sample_edit(skill_id: &str) -> CommitSkillEdit {
        CommitSkillEdit {
            skill_id: skill_id.to_owned(),
            remote_id: None,
            name: "Outcomes Skill".to_owned(),
            slug: "outcomes-skill".to_owned(),
            workspace_path: std::env::temp_dir().join("skillhive-outcomes"),
            blob_hash: "sha256:abc123".to_owned(),
            base_revision: None,
            operation: MutationOperation::Create,
        }
    }

    fn ack_outcome(mutation_id: &str, revision: i64) -> MutationOutcome {
        MutationOutcome {
            mutation_id: mutation_id.to_owned(),
            status: "acked".to_owned(),
            remote_skill_id: Some("remote-1".to_owned()),
            revision: Some(revision),
            conflict_head_revision: None,
            error_code: None,
            message: None,
        }
    }

    #[test]
    fn acked_outcome_maps_remote_id_and_syncs_skill() {
        let (_temp, store) = open_temp_store();
        let mutation = store
            .commit_skill_edit(sample_edit("skill-1"))
            .expect("commit");

        let applied = store
            .apply_mutation_outcome(&ack_outcome(&mutation.id, 1))
            .expect("apply");
        assert_eq!(applied.mutation_state, MutationState::Acked);
        assert_eq!(applied.skill_state, Some(SkillSyncState::Synced));

        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.remote_id.as_deref(), Some("remote-1"));
        assert_eq!(skill.remote_revision, Some(1));
        assert_eq!(skill.sync_state, SkillSyncState::Synced);
    }

    #[test]
    fn acked_create_keeps_skill_dirty_while_later_mutation_pending() {
        let (_temp, store) = open_temp_store();
        let create = store
            .commit_skill_edit(sample_edit("skill-1"))
            .expect("create");
        let mut edit = sample_edit("skill-1");
        edit.operation = MutationOperation::Update;
        let update = store.commit_skill_edit(edit).expect("update");

        store
            .apply_mutation_outcome(&ack_outcome(&create.id, 1))
            .expect("ack create");

        // The update mutation is still pending, so the Skill head stays dirty.
        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Dirty);
        assert_eq!(skill.remote_id.as_deref(), Some("remote-1"));

        store
            .apply_mutation_outcome(&ack_outcome(&update.id, 2))
            .expect("ack update");
        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Synced);
        assert_eq!(skill.remote_revision, Some(2));
    }

    #[test]
    fn conflict_outcome_marks_conflict_and_keeps_snapshot() {
        let (_temp, store) = open_temp_store();
        let mutation = store
            .commit_skill_edit(sample_edit("skill-1"))
            .expect("commit");

        let applied = store
            .apply_mutation_outcome(&MutationOutcome {
                mutation_id: mutation.id.clone(),
                status: "conflict".to_owned(),
                remote_skill_id: Some("remote-1".to_owned()),
                revision: None,
                conflict_head_revision: Some(9),
                error_code: Some("REVISION_CONFLICT".to_owned()),
                message: Some("server head advanced".to_owned()),
            })
            .expect("apply");

        assert_eq!(applied.mutation_state, MutationState::Conflict);
        assert_eq!(applied.skill_state, Some(SkillSyncState::Conflict));
        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Conflict);
        assert_eq!(skill.current_blob_hash, "sha256:abc123");
    }

    #[test]
    fn transport_error_persists_backoff_schedule() {
        let (_temp, store) = open_temp_store();
        let mutation = store
            .commit_skill_edit(sample_edit("skill-1"))
            .expect("commit");

        let applied = store
            .apply_mutation_outcome(&MutationOutcome::transport_error(
                &mutation.id,
                "NETWORK_UNAVAILABLE",
                "connection refused",
            ))
            .expect("apply");
        assert_eq!(applied.mutation_state, MutationState::RetryableError);

        // Not dispatchable until the persisted backoff window elapses.
        assert!(store
            .list_dispatchable_mutations(10)
            .expect("dispatchable")
            .is_empty());
    }

    #[test]
    fn validation_error_is_permanent_not_retryable() {
        let (_temp, store) = open_temp_store();
        let mutation = store
            .commit_skill_edit(sample_edit("skill-1"))
            .expect("commit");

        let applied = store
            .apply_mutation_outcome(&MutationOutcome {
                mutation_id: mutation.id.clone(),
                status: "validation_error".to_owned(),
                remote_skill_id: None,
                revision: None,
                conflict_head_revision: None,
                error_code: Some("SKILL_SLUG_TAKEN".to_owned()),
                message: Some("slug taken".to_owned()),
            })
            .expect("apply");
        assert_eq!(applied.mutation_state, MutationState::PermanentError);

        // Permanent errors never re-enter dispatch, preventing retry storms.
        assert!(store
            .list_dispatchable_mutations(10)
            .expect("dispatchable")
            .is_empty());
    }

    #[test]
    fn permission_denied_preserves_local_work() {
        let (_temp, store) = open_temp_store();
        let mutation = store
            .commit_skill_edit(sample_edit("skill-1"))
            .expect("commit");

        store
            .apply_mutation_outcome(&MutationOutcome {
                mutation_id: mutation.id.clone(),
                status: "permission_denied".to_owned(),
                remote_skill_id: None,
                revision: None,
                conflict_head_revision: None,
                error_code: Some("SKILL_NOT_FOUND".to_owned()),
                message: Some("not found".to_owned()),
            })
            .expect("apply");

        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::AccessRevoked);
        assert_eq!(skill.current_blob_hash, "sha256:abc123");
    }

    #[test]
    fn unknown_mutation_id_is_rejected() {
        let (_temp, store) = open_temp_store();
        let error = store
            .apply_mutation_outcome(&ack_outcome("missing-mutation", 1))
            .expect_err("must fail");
        assert!(matches!(error, LocalStoreError::OutboxClaimLost(_)));
    }

    #[test]
    fn backoff_is_bounded() {
        assert_eq!(backoff_seconds(0), 1);
        assert_eq!(backoff_seconds(1), 2);
        assert_eq!(backoff_seconds(3), 8);
        assert_eq!(backoff_seconds(6), 64);
        assert_eq!(backoff_seconds(u32::MAX), BACKOFF_MAX_SECONDS);
    }
}
