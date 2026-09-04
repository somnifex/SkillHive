use std::path::PathBuf;

use rusqlite::{params, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{LocalStore, LocalStoreError, SkillSyncState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalCachePolicy {
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSkillRecord {
    pub skill_id: String,
    pub remote_id: Option<String>,
    pub workspace_path: PathBuf,
    pub snapshot_hash: String,
    pub sync_state: SkillSyncState,
    pub pinned: bool,
    pub last_accessed_at: String,
}

impl LocalStore {
    pub fn cache_policy(&self) -> Result<LocalCachePolicy, LocalStoreError> {
        let connection = self.lock_connection()?;
        let max_bytes: i64 = connection.query_row(
            "SELECT max_bytes FROM local_cache_policy WHERE id = 1",
            [],
            |row| row.get(0),
        )?;
        let max_bytes = u64::try_from(max_bytes).map_err(|_| {
            LocalStoreError::InvalidPersistedState("local cache max_bytes is negative".to_owned())
        })?;
        Ok(LocalCachePolicy { max_bytes })
    }

    pub fn set_cache_policy(
        &self,
        policy: LocalCachePolicy,
    ) -> Result<LocalCachePolicy, LocalStoreError> {
        if policy.max_bytes == 0 || policy.max_bytes > i64::MAX as u64 {
            return Err(LocalStoreError::InvalidInput(
                "local cache max_bytes must be between 1 and i64::MAX".to_owned(),
            ));
        }
        let connection = self.lock_connection()?;
        connection.execute(
            r#"
            UPDATE local_cache_policy
            SET max_bytes = ?1, updated_at = CURRENT_TIMESTAMP
            WHERE id = 1
            "#,
            [policy.max_bytes as i64],
        )?;
        Ok(policy)
    }

    pub fn list_cache_skills(&self) -> Result<Vec<CacheSkillRecord>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT id, remote_id, workspace_path, current_blob_hash, sync_state,
                   pinned, COALESCE(last_accessed_at, updated_at, created_at)
            FROM local_skills
            ORDER BY COALESCE(last_accessed_at, updated_at, created_at) ASC, id ASC
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, bool>(5)?,
                row.get::<_, String>(6)?,
            ))
        })?;

        rows.map(|row| {
            let row = row?;
            Ok(CacheSkillRecord {
                skill_id: row.0,
                remote_id: row.1,
                workspace_path: PathBuf::from(row.2),
                snapshot_hash: row.3,
                sync_state: SkillSyncState::from_db_str(&row.4)?,
                pinned: row.5,
                last_accessed_at: row.6,
            })
        })
        .collect()
    }

    /// Returns snapshot payloads that are still required to complete or resolve
    /// an outbox mutation. CacheManager must treat every returned hash as a
    /// strong root even when it is no longer the skill's current snapshot.
    pub fn list_unacked_mutation_payload_hashes(&self) -> Result<Vec<String>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT DISTINCT payload_hash
            FROM local_mutations
            WHERE state <> 'acked'
            ORDER BY payload_hash ASC
            "#,
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn claim_skill_for_eviction(
        &self,
        skill_id: &str,
        expected_snapshot_hash: &str,
    ) -> Result<bool, LocalStoreError> {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            r#"
            UPDATE local_skills
            SET sync_state = 'remote_only', updated_at = CURRENT_TIMESTAMP
            WHERE id = ?1
              AND current_blob_hash = ?2
              AND remote_id IS NOT NULL
              AND sync_state = 'synced'
              AND pinned = 0
              AND NOT EXISTS (
                  SELECT 1 FROM skill_deployments d
                  WHERE d.skill_id = local_skills.id
                    AND d.state IN (
                        'installing', 'installed', 'updating', 'modified',
                        'removing', 'revoked'
                    )
              )
            "#,
            params![skill_id, expected_snapshot_hash],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_policy_is_persistent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("skillhive.db");
        {
            let store = LocalStore::open(&path).expect("store");
            assert_eq!(store.cache_policy().expect("policy").max_bytes, 2 * 1024 * 1024 * 1024);
            store
                .set_cache_policy(LocalCachePolicy { max_bytes: 123_456 })
                .expect("set");
        }
        let reopened = LocalStore::open(&path).expect("reopen");
        assert_eq!(reopened.cache_policy().expect("policy").max_bytes, 123_456);
    }
}
