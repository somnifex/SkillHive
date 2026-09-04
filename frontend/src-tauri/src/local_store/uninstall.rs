use std::path::Path;

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{path_to_string, LocalStore, LocalStoreError};

impl LocalStore {
    /// Removes exactly the deployment catalog row that the filesystem uninstall
    /// transaction was started against. Hash/path matching prevents a stale UI
    /// request from deleting a newer deployment record.
    pub fn remove_deployment_catalog(
        &self,
        skill_id: &str,
        agent_profile_id: &str,
        expected_snapshot_hash: &str,
        expected_target_path: &Path,
    ) -> Result<bool, LocalStoreError> {
        let expected_target = path_to_string(expected_target_path)?;
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: Option<(String, String)> = transaction
            .query_row(
                r#"
                SELECT deployed_blob_hash, target_path
                FROM skill_deployments
                WHERE skill_id = ?1 AND agent_profile_id = ?2
                "#,
                params![skill_id, agent_profile_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((current_hash, current_target)) = current else {
            transaction.commit()?;
            return Ok(false);
        };
        if current_hash != expected_snapshot_hash || current_target != expected_target {
            return Err(LocalStoreError::DeploymentCatalogChanged {
                skill_id: skill_id.to_owned(),
                agent_profile_id: agent_profile_id.to_owned(),
            });
        }

        let deleted = transaction.execute(
            "DELETE FROM skill_deployments WHERE skill_id = ?1 AND agent_profile_id = ?2",
            params![skill_id, agent_profile_id],
        )?;
        transaction.commit()?;
        Ok(deleted == 1)
    }
}
