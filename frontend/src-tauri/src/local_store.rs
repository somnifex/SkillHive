mod cache;
mod deployments;
mod migrations;
mod mutations;
mod skills;

pub use cache::{CacheSkillRecord, LocalCachePolicy};

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct LocalStore {
    connection: Mutex<Connection>,
    db_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSyncState {
    RemoteOnly,
    Synced,
    Dirty,
    Uploading,
    Conflict,
    SyncError,
    AccessRevoked,
    Corrupted,
}

impl SkillSyncState {
    pub(super) fn from_db_str(value: &str) -> Result<Self, LocalStoreError> {
        match value {
            "remote_only" => Ok(Self::RemoteOnly),
            "synced" => Ok(Self::Synced),
            "dirty" => Ok(Self::Dirty),
            "uploading" => Ok(Self::Uploading),
            "conflict" => Ok(Self::Conflict),
            "sync_error" => Ok(Self::SyncError),
            "access_revoked" => Ok(Self::AccessRevoked),
            "corrupted" => Ok(Self::Corrupted),
            other => Err(LocalStoreError::InvalidPersistedState(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationOperation {
    Create,
    Update,
    Delete,
}

impl MutationOperation {
    pub(super) fn as_db_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }

    pub(super) fn from_db_str(value: &str) -> Result<Self, LocalStoreError> {
        match value {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            other => Err(LocalStoreError::InvalidPersistedState(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationState {
    Pending,
    InFlight,
    Acked,
    RetryableError,
    Conflict,
    PermissionDenied,
    PermanentError,
}

impl MutationState {
    pub(super) fn from_db_str(value: &str) -> Result<Self, LocalStoreError> {
        match value {
            "pending" => Ok(Self::Pending),
            "in_flight" => Ok(Self::InFlight),
            "acked" => Ok(Self::Acked),
            "retryable_error" => Ok(Self::RetryableError),
            "conflict" => Ok(Self::Conflict),
            "permission_denied" => Ok(Self::PermissionDenied),
            "permanent_error" => Ok(Self::PermanentError),
            other => Err(LocalStoreError::InvalidPersistedState(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentState {
    Installing,
    Installed,
    Updating,
    Removing,
    Modified,
    Missing,
    Failed,
    Revoked,
}

impl DeploymentState {
    pub(super) fn as_db_str(self) -> &'static str {
        match self {
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Updating => "updating",
            Self::Removing => "removing",
            Self::Modified => "modified",
            Self::Missing => "missing",
            Self::Failed => "failed",
            Self::Revoked => "revoked",
        }
    }

    pub(super) fn from_db_str(value: &str) -> Result<Self, LocalStoreError> {
        match value {
            "installing" => Ok(Self::Installing),
            "installed" => Ok(Self::Installed),
            "updating" => Ok(Self::Updating),
            "removing" => Ok(Self::Removing),
            "modified" => Ok(Self::Modified),
            "missing" => Ok(Self::Missing),
            "failed" => Ok(Self::Failed),
            "revoked" => Ok(Self::Revoked),
            other => Err(LocalStoreError::InvalidPersistedState(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CommitSkillEdit {
    pub skill_id: String,
    pub remote_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub workspace_path: PathBuf,
    pub blob_hash: String,
    pub base_revision: Option<i64>,
    pub operation: MutationOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSkill {
    pub id: String,
    pub remote_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub workspace_path: PathBuf,
    pub current_blob_hash: String,
    pub remote_revision: Option<i64>,
    pub sync_state: SkillSyncState,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalMutation {
    pub id: String,
    pub skill_id: String,
    pub operation: MutationOperation,
    pub base_revision: Option<i64>,
    pub payload_hash: String,
    pub state: MutationState,
    pub retry_count: u32,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpsertAgentProfile {
    pub id: String,
    pub descriptor_id: String,
    pub display_name: String,
    pub skill_root: PathBuf,
    pub enabled: bool,
    pub is_custom: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProfileRecord {
    pub id: String,
    pub descriptor_id: String,
    pub display_name: String,
    pub skill_root: PathBuf,
    pub enabled: bool,
    pub is_custom: bool,
}

#[derive(Debug, Clone)]
pub struct RecordDeployment {
    pub skill_id: String,
    pub agent_profile_id: String,
    pub deployed_blob_hash: String,
    pub target_path: PathBuf,
    pub state: DeploymentState,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDeploymentRecord {
    pub skill_id: String,
    pub agent_profile_id: String,
    pub deployed_blob_hash: String,
    pub target_path: PathBuf,
    pub state: DeploymentState,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStoreHealth {
    pub schema_version: i64,
    pub pending_mutations: u64,
}

impl LocalStore {
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, LocalStoreError> {
        let db_path = db_path.as_ref().to_path_buf();
        if let Some(parent) = db_path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }

        let mut connection = Connection::open(&db_path)?;
        configure_connection(&connection)?;
        migrations::migrate(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
            db_path,
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn health(&self) -> Result<LocalStoreHealth, LocalStoreError> {
        let connection = self.lock_connection()?;
        let schema_version = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        let pending_mutations: i64 = connection.query_row(
            "SELECT COUNT(*) FROM local_mutations WHERE state IN ('pending', 'retryable_error')",
            [],
            |row| row.get(0),
        )?;

        Ok(LocalStoreHealth {
            schema_version,
            pending_mutations: pending_mutations.try_into().unwrap_or_default(),
        })
    }

    pub(super) fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, LocalStoreError> {
        self.connection
            .lock()
            .map_err(|_| LocalStoreError::LockPoisoned)
    }
}

fn configure_connection(connection: &Connection) -> Result<(), LocalStoreError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        PRAGMA wal_autocheckpoint = 1000;
        "#,
    )?;
    Ok(())
}

pub(super) fn path_to_string(path: &Path) -> Result<String, LocalStoreError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| LocalStoreError::NonUtf8Path(path.to_path_buf()))
}

pub(super) fn validate_non_empty(field: &str, value: &str) -> Result<(), LocalStoreError> {
    if value.trim().is_empty() {
        return Err(LocalStoreError::InvalidInput(format!(
            "{field} must not be empty"
        )));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum LocalStoreError {
    #[error("local database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("local filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("local database lock was poisoned")]
    LockPoisoned,
    #[error("database schema version {found} is newer than supported version {supported}")]
    UnsupportedSchema { found: i64, supported: i64 },
    #[error("invalid persisted state: {0}")]
    InvalidPersistedState(String),
    #[error("invalid local store input: {0}")]
    InvalidInput(String),
    #[error("skill not found: {0}")]
    SkillNotFound(String),
    #[error("agent profile not found: {0}")]
    AgentProfileNotFound(String),
    #[error(
        "agent profile {profile_id} cannot move from {current_root:?} to {requested_root:?} while {deployment_count} deployments exist"
    )]
    AgentProfileRootInUse {
        profile_id: String,
        current_root: PathBuf,
        requested_root: PathBuf,
        deployment_count: u64,
    },
    #[error("path is not valid UTF-8: {0:?}")]
    NonUtf8Path(PathBuf),
}
