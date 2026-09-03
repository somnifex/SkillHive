use std::path::PathBuf;

use rusqlite::{params, OptionalExtension, TransactionBehavior};

use super::{
    path_to_string, validate_non_empty, AgentProfileRecord, DeploymentState, LocalStore,
    LocalStoreError, RecordDeployment, SkillDeploymentRecord, UpsertAgentProfile,
};

impl LocalStore {
    pub fn upsert_agent_profile(
        &self,
        profile: UpsertAgentProfile,
    ) -> Result<AgentProfileRecord, LocalStoreError> {
        validate_agent_profile(&profile)?;
        let skill_root = path_to_string(&profile.skill_root)?;
        let connection = self.lock_connection()?;
        connection.execute(
            r#"
            INSERT INTO agent_profiles(
                id, descriptor_id, display_name, skill_root, enabled, is_custom, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP)
            ON CONFLICT(id) DO UPDATE SET
                descriptor_id = excluded.descriptor_id,
                display_name = excluded.display_name,
                skill_root = excluded.skill_root,
                enabled = excluded.enabled,
                is_custom = excluded.is_custom,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                profile.id,
                profile.descriptor_id,
                profile.display_name,
                skill_root,
                profile.enabled,
                profile.is_custom,
            ],
        )?;

        Ok(AgentProfileRecord {
            id: profile.id,
            descriptor_id: profile.descriptor_id,
            display_name: profile.display_name,
            skill_root: profile.skill_root,
            enabled: profile.enabled,
            is_custom: profile.is_custom,
        })
    }

    pub fn list_agent_profiles(&self) -> Result<Vec<AgentProfileRecord>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(
            r#"
            SELECT id, descriptor_id, display_name, skill_root, enabled, is_custom
            FROM agent_profiles
            ORDER BY display_name ASC, id ASC
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AgentProfileRecord {
                id: row.get(0)?,
                descriptor_id: row.get(1)?,
                display_name: row.get(2)?,
                skill_root: PathBuf::from(row.get::<_, String>(3)?),
                enabled: row.get(4)?,
                is_custom: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Records the filesystem deployment outcome. The target must be a direct
    /// child of the persisted agent skill root, preventing a mismatched profile
    /// id from recording an unrelated filesystem path.
    pub fn record_deployment(
        &self,
        deployment: RecordDeployment,
    ) -> Result<SkillDeploymentRecord, LocalStoreError> {
        validate_non_empty("skill_id", &deployment.skill_id)?;
        validate_non_empty("agent_profile_id", &deployment.agent_profile_id)?;
        validate_non_empty("deployed_blob_hash", &deployment.deployed_blob_hash)?;
        if !deployment.target_path.is_absolute() {
            return Err(LocalStoreError::InvalidInput(
                "deployment target_path must be absolute".to_owned(),
            ));
        }
        let target_path = path_to_string(&deployment.target_path)?;

        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let profile_root: Option<String> = transaction
            .query_row(
                "SELECT skill_root FROM agent_profiles WHERE id = ?1",
                [deployment.agent_profile_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let profile_root = profile_root.ok_or_else(|| {
            LocalStoreError::AgentProfileNotFound(deployment.agent_profile_id.clone())
        })?;
        let profile_root = PathBuf::from(profile_root);
        if deployment.target_path.parent() != Some(profile_root.as_path()) {
            return Err(LocalStoreError::InvalidInput(format!(
                "deployment target {} is not a direct child of agent profile root {}",
                deployment.target_path.display(),
                profile_root.display()
            )));
        }

        transaction.execute(
            r#"
            INSERT INTO skill_deployments(
                skill_id, agent_profile_id, deployed_blob_hash, target_path, state,
                last_error, last_verified_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)
            ON CONFLICT(skill_id, agent_profile_id) DO UPDATE SET
                deployed_blob_hash = excluded.deployed_blob_hash,
                target_path = excluded.target_path,
                state = excluded.state,
                last_error = excluded.last_error,
                last_verified_at = CURRENT_TIMESTAMP,
                updated_at = CURRENT_TIMESTAMP
            "#,
            params![
                deployment.skill_id,
                deployment.agent_profile_id,
                deployment.deployed_blob_hash,
                target_path,
                deployment.state.as_db_str(),
                deployment.last_error,
            ],
        )?;
        transaction.commit()?;

        Ok(SkillDeploymentRecord {
            skill_id: deployment.skill_id,
            agent_profile_id: deployment.agent_profile_id,
            deployed_blob_hash: deployment.deployed_blob_hash,
            target_path: deployment.target_path,
            state: deployment.state,
            last_error: deployment.last_error,
        })
    }

    pub fn get_deployment(
        &self,
        skill_id: &str,
        agent_profile_id: &str,
    ) -> Result<Option<SkillDeploymentRecord>, LocalStoreError> {
        let connection = self.lock_connection()?;
        let row = connection
            .query_row(
                r#"
                SELECT skill_id, agent_profile_id, deployed_blob_hash, target_path,
                       state, last_error
                FROM skill_deployments
                WHERE skill_id = ?1 AND agent_profile_id = ?2
                "#,
                params![skill_id, agent_profile_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;

        row.map(|row| {
            Ok(SkillDeploymentRecord {
                skill_id: row.0,
                agent_profile_id: row.1,
                deployed_blob_hash: row.2,
                target_path: PathBuf::from(row.3),
                state: DeploymentState::from_db_str(&row.4)?,
                last_error: row.5,
            })
        })
        .transpose()
    }
}

fn validate_agent_profile(profile: &UpsertAgentProfile) -> Result<(), LocalStoreError> {
    for (field, value) in [
        ("id", profile.id.as_str()),
        ("descriptor_id", profile.descriptor_id.as_str()),
        ("display_name", profile.display_name.as_str()),
    ] {
        validate_non_empty(field, value)?;
    }
    if !profile.skill_root.is_absolute() {
        return Err(LocalStoreError::InvalidInput(
            "agent skill_root must be absolute".to_owned(),
        ));
    }
    path_to_string(&profile.skill_root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_store::{
        CommitSkillEdit, MutationOperation, RecordDeployment, UpsertAgentProfile,
    };

    fn store_with_skill() -> (tempfile::TempDir, LocalStore) {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("store");
        store
            .commit_skill_edit(CommitSkillEdit {
                skill_id: "skill-1".to_owned(),
                remote_id: None,
                name: "Code Review".to_owned(),
                slug: "code-review".to_owned(),
                workspace_path: temp.path().join("workspace").join("code-review"),
                blob_hash: "sha256:abc".to_owned(),
                base_revision: None,
                operation: MutationOperation::Create,
            })
            .expect("skill");
        (temp, store)
    }

    #[test]
    fn profile_and_deployment_are_persisted() {
        let (temp, store) = store_with_skill();
        let root = temp.path().join("claude-skills");
        let profile = store
            .upsert_agent_profile(UpsertAgentProfile {
                id: "claude-code:default".to_owned(),
                descriptor_id: "claude-code".to_owned(),
                display_name: "Claude Code".to_owned(),
                skill_root: root.clone(),
                enabled: true,
                is_custom: false,
            })
            .expect("profile");
        assert_eq!(profile.skill_root, root);

        let deployment = store
            .record_deployment(RecordDeployment {
                skill_id: "skill-1".to_owned(),
                agent_profile_id: profile.id.clone(),
                deployed_blob_hash: "sha256:abc".to_owned(),
                target_path: profile.skill_root.join("code-review"),
                state: DeploymentState::Installed,
                last_error: None,
            })
            .expect("deployment");
        assert_eq!(deployment.state, DeploymentState::Installed);
        assert_eq!(
            store
                .get_deployment("skill-1", &profile.id)
                .expect("get")
                .expect("deployment"),
            deployment
        );
    }

    #[test]
    fn deployment_target_must_match_profile_root() {
        let (temp, store) = store_with_skill();
        store
            .upsert_agent_profile(UpsertAgentProfile {
                id: "custom:test".to_owned(),
                descriptor_id: "custom:test".to_owned(),
                display_name: "Test".to_owned(),
                skill_root: temp.path().join("allowed"),
                enabled: true,
                is_custom: true,
            })
            .expect("profile");

        let result = store.record_deployment(RecordDeployment {
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "custom:test".to_owned(),
            deployed_blob_hash: "sha256:abc".to_owned(),
            target_path: temp.path().join("elsewhere").join("code-review"),
            state: DeploymentState::Installed,
            last_error: None,
        });
        assert!(matches!(result, Err(LocalStoreError::InvalidInput(_))));
    }
}
