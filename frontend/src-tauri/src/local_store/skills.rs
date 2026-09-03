use std::path::PathBuf;

use rusqlite::OptionalExtension;

use super::{LocalSkill, LocalStore, LocalStoreError, SkillSyncState};

impl LocalStore {
    pub fn get_skill(&self, skill_id: &str) -> Result<Option<LocalSkill>, LocalStoreError> {
        let connection = self.lock_connection()?;
        connection
            .query_row(
                r#"
                SELECT id, remote_id, name, slug, workspace_path, current_blob_hash,
                       remote_revision, sync_state, pinned
                FROM local_skills
                WHERE id = ?1
                "#,
                [skill_id],
                |row| {
                    let sync_state: String = row.get(7)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        sync_state,
                        row.get::<_, bool>(8)?,
                    ))
                },
            )
            .optional()?
            .map(|row| {
                Ok(LocalSkill {
                    id: row.0,
                    remote_id: row.1,
                    name: row.2,
                    slug: row.3,
                    workspace_path: PathBuf::from(row.4),
                    current_blob_hash: row.5,
                    remote_revision: row.6,
                    sync_state: SkillSyncState::from_db_str(&row.7)?,
                    pinned: row.8,
                })
            })
            .transpose()
    }
}
