use std::{
    fs,
    path::{Component, Path},
};

use serde::{Deserialize, Serialize};

use crate::{
    agent::{validate_persisted_profile, validate_skill_root, AgentAdapterError},
    blob_store::BlobStore,
    local_store::{LocalStore, LocalStoreError},
    skill_snapshot::{capture_workspace, SkillSnapshotRef, SnapshotError, SnapshotPolicy},
};

use super::{WorkspaceError, WorkspaceRef, WorkspaceStore};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAgentSkillRequest {
    pub skill_id: String,
    pub agent_profile_id: String,
    pub directory_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportAgentSkillResult {
    pub workspace: WorkspaceRef,
    pub snapshot: SkillSnapshotRef,
    pub source_profile_id: String,
    pub source_directory_name: String,
}

/// Imports an existing skill from a persisted Agent profile into SkillHive's
/// managed workspace without accepting an arbitrary source path from the UI.
///
/// The source path is always derived as `<persisted skill_root>/<directory>`.
/// Snapshot capture rejects symlinks, traversal, nonportable filenames and size
/// policy violations before any managed workspace is materialized.
pub fn import_agent_skill(
    store: &LocalStore,
    blobs: &BlobStore,
    workspaces: &WorkspaceStore,
    request: ImportAgentSkillRequest,
) -> Result<ImportAgentSkillResult, AgentSkillImportError> {
    validate_directory_name(&request.directory_name)?;
    let profile = store
        .get_agent_profile(&request.agent_profile_id)?
        .ok_or_else(|| AgentSkillImportError::ProfileNotFound(request.agent_profile_id.clone()))?;
    if !profile.enabled {
        return Err(AgentSkillImportError::ProfileDisabled(profile.id));
    }

    // Revalidate persisted identity/path before every privileged read. This
    // treats accidental/corrupt SQLite state as untrusted rather than assuming
    // that every row must have passed the original insertion boundary.
    validate_persisted_profile(
        &profile.id,
        &profile.descriptor_id,
        &profile.skill_root,
        profile.is_custom,
    )?;
    validate_skill_root(&profile.skill_root)?;

    let source = profile.skill_root.join(&request.directory_name);
    if source.parent() != Some(profile.skill_root.as_path()) {
        return Err(AgentSkillImportError::InvalidDirectoryName(
            request.directory_name,
        ));
    }

    let metadata = fs::symlink_metadata(&source).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AgentSkillImportError::SourceNotFound(source.clone())
        } else {
            AgentSkillImportError::Io(error)
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(AgentSkillImportError::InvalidSource(source));
    }

    let snapshot = capture_workspace(blobs, &source, SnapshotPolicy::default())?;
    let workspace = workspaces.import_snapshot(blobs, &request.skill_id, &snapshot.manifest_hash)?;

    Ok(ImportAgentSkillResult {
        workspace,
        snapshot,
        source_profile_id: profile.id,
        source_directory_name: request.directory_name,
    })
}

fn validate_directory_name(value: &str) -> Result<(), AgentSkillImportError> {
    if value.is_empty() || value.starts_with(".skillhive-") {
        return Err(AgentSkillImportError::InvalidDirectoryName(value.to_owned()));
    }
    let mut components = Path::new(value).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) if !component.is_empty() => Ok(()),
        _ => Err(AgentSkillImportError::InvalidDirectoryName(value.to_owned())),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentSkillImportError {
    #[error("agent profile not found: {0}")]
    ProfileNotFound(String),
    #[error("agent profile is disabled: {0}")]
    ProfileDisabled(String),
    #[error("invalid agent skill directory name: {0}")]
    InvalidDirectoryName(String),
    #[error("agent skill source not found: {0:?}")]
    SourceNotFound(std::path::PathBuf),
    #[error("invalid agent skill source: {0:?}")]
    InvalidSource(std::path::PathBuf),
    #[error("agent skill import filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("agent adapter error: {0}")]
    Agent(#[from] AgentAdapterError),
    #[error("local store error: {0}")]
    LocalStore(#[from] LocalStoreError),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("workspace error: {0}")]
    Workspace(#[from] WorkspaceError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_store::UpsertAgentProfile;

    #[test]
    fn import_source_is_derived_from_profile_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("store");
        let root = temp.path().join("agent-skills");
        let source = root.join("existing-skill");
        fs::create_dir_all(&source).expect("source");
        fs::write(source.join("SKILL.md"), b"# Existing\n").expect("skill");
        store
            .upsert_agent_profile(UpsertAgentProfile {
                id: "custom:test:configured".to_owned(),
                descriptor_id: "custom:test".to_owned(),
                display_name: "Test Agent".to_owned(),
                skill_root: root,
                enabled: true,
                is_custom: true,
            })
            .expect("profile");

        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");
        let workspaces = WorkspaceStore::open(temp.path().join("workspaces")).expect("workspaces");
        let result = import_agent_skill(
            &store,
            &blobs,
            &workspaces,
            ImportAgentSkillRequest {
                skill_id: "skill-1".to_owned(),
                agent_profile_id: "custom:test:configured".to_owned(),
                directory_name: "existing-skill".to_owned(),
            },
        )
        .expect("import");

        assert_eq!(
            fs::read(result.workspace.path.join("SKILL.md")).expect("read"),
            b"# Existing\n"
        );
    }

    #[test]
    fn traversal_directory_is_rejected_before_filesystem_access() {
        assert!(matches!(
            validate_directory_name("../escape"),
            Err(AgentSkillImportError::InvalidDirectoryName(_))
        ));
    }
}
