use std::path::PathBuf;

use rusqlite::OptionalExtension;

use super::{LocalSkill, LocalStore, LocalStoreError, SkillSyncState};

impl LocalStore {
    /// Reads a local skill and updates its local cache access timestamp.
    ///
    /// `last_accessed_at` is deliberately local-only metadata. It never
    /// participates in sync conflict resolution or authorization decisions.
    pub fn get_skill(&self, skill_id: &str) -> Result<Option<LocalSkill>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let row = connection
            .query_row(
                r#"
                SELECT id, remote_id, name, slug, workspace_path, current_blob_hash,
                       remote_blob_hash, remote_revision, sync_state, pinned
                FROM local_skills
                WHERE id = ?1
                "#,
                [skill_id],
                |row| {
                    let sync_state: String = row.get(8)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        sync_state,
                        row.get::<_, bool>(9)?,
                    ))
                },
            )
            .optional()?;

        let Some(row) = row else {
            return Ok(None);
        };

        connection.execute(
            "UPDATE local_skills SET last_accessed_at = CURRENT_TIMESTAMP WHERE id = ?1",
            [skill_id],
        )?;

        Ok(Some(LocalSkill {
            id: row.0,
            remote_id: row.1,
            name: row.2,
            slug: row.3,
            workspace_path: PathBuf::from(row.4),
            current_blob_hash: row.5,
            remote_blob_hash: row.6,
            remote_revision: row.7,
            sync_state: SkillSyncState::from_db_str(&row.8)?,
            pinned: row.9,
        }))
    }

    /// Promotes a hydrated remote-only record: the snapshot closure is now
    /// fully local and a managed workspace exists, so the record behaves
    /// like a synced mirror of the server skill.
    ///
    /// The manifest hash is part of the compare-and-swap predicate. Pull
    /// apply is the other writer of `current_blob_hash`; if it wins the race
    /// this update affects zero rows and hydration retries the newer head
    /// instead of falsely marking an old workspace as synced.
    pub fn mark_skill_hydrated(
        &self,
        skill_id: &str,
        expected_manifest_hash: &str,
        workspace_path: &std::path::Path,
    ) -> Result<LocalSkill, LocalStoreError> {
        let mut connection = self.lock_connection()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let workspace_path = super::path_to_string(workspace_path)?;
        let changed = transaction.execute(
            r#"
            UPDATE local_skills
            SET workspace_path = ?1, sync_state = 'synced', updated_at = CURRENT_TIMESTAMP
            WHERE id = ?2
              AND sync_state IN ('remote_only', 'synced')
              AND current_blob_hash = ?3
            "#,
            rusqlite::params![workspace_path, skill_id, expected_manifest_hash],
        )?;
        if changed == 0 {
            let existing: Option<String> = transaction
                .query_row(
                    "SELECT sync_state FROM local_skills WHERE id = ?1",
                    [skill_id],
                    |row| row.get(0),
                )
                .optional()?;
            return Err(match existing {
                None => LocalStoreError::SkillNotFound(skill_id.to_owned()),
                Some(state) => LocalStoreError::InvalidInput(format!(
                    "skill {skill_id} in state {state} cannot be hydrated"
                )),
            });
        }
        transaction.commit()?;
        drop(connection);

        self.get_skill(skill_id)?
            .ok_or_else(|| LocalStoreError::SkillNotFound(skill_id.to_owned()))
    }

    /// Resolves a local skill row by its server-assigned id. The UI's skill
    /// lists come from the server, so export flows translate the remote id
    /// into the local row that carries the snapshot hash.
    pub fn get_skill_by_remote_id(
        &self,
        remote_id: &str,
    ) -> Result<Option<LocalSkill>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let row = connection
            .query_row(
                r#"
                SELECT id, remote_id, name, slug, workspace_path, current_blob_hash,
                       remote_blob_hash, remote_revision, sync_state, pinned
                FROM local_skills
                WHERE remote_id = ?1
                "#,
                [remote_id],
                |row| {
                    let sync_state: String = row.get(8)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<i64>>(7)?,
                        sync_state,
                        row.get::<_, bool>(9)?,
                    ))
                },
            )
            .optional()?;

        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(LocalSkill {
            id: row.0,
            remote_id: row.1,
            name: row.2,
            slug: row.3,
            workspace_path: PathBuf::from(row.4),
            current_blob_hash: row.5,
            remote_blob_hash: row.6,
            remote_revision: row.7,
            sync_state: SkillSyncState::from_db_str(&row.8)?,
            pinned: row.9,
        }))
    }
}
