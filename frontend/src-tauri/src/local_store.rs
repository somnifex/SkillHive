mod migrations;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    fn as_db_str(self) -> &'static str {
        match self {
            Self::RemoteOnly => "remote_only",
            Self::Synced => "synced",
            Self::Dirty => "dirty",
            Self::Uploading => "uploading",
            Self::Conflict => "conflict",
            Self::SyncError => "sync_error",
            Self::AccessRevoked => "access_revoked",
            Self::Corrupted => "corrupted",
        }
    }

    fn from_db_str(value: &str) -> Result<Self, LocalStoreError> {
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
    fn as_db_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }

    fn from_db_str(value: &str) -> Result<Self, LocalStoreError> {
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
    fn from_db_str(value: &str) -> Result<Self, LocalStoreError> {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStoreHealth {
    pub schema_version: i64,
    pub pending_mutations: u64,
}

impl LocalStore {
    /// Opens the desktop database and applies forward-only schema migrations.
    ///
    /// The connection is intentionally serialized behind a mutex. Desktop write
    /// volume is low, and a single writer makes transaction ownership explicit
    /// while SQLite WAL still permits concurrent readers at the database level.
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

    /// Queues deletion without removing the local record. The row acts as a
    /// local tombstone source until the server acknowledges the mutation.
    pub fn queue_skill_delete(
        &self,
        skill_id: &str,
        base_revision: Option<i64>,
    ) -> Result<LocalMutation, LocalStoreError> {
        if skill_id.trim().is_empty() {
            return Err(LocalStoreError::InvalidInput(
                "skill_id must not be empty".to_owned(),
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
        let payload_hash = payload_hash.ok_or_else(|| LocalStoreError::SkillNotFound(skill_id.to_owned()))?;

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

    pub fn list_dispatchable_mutations(&self, limit: u32) -> Result<Vec<LocalMutation>, LocalStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }

        let connection = self.lock_connection()?;
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

    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, LocalStoreError> {
        self.connection
            .lock()
            .map_err(|_| LocalStoreError::LockPoisoned)
    }
}

fn configure_connection(connection: &Connection) -> Result<(), LocalStoreError> {
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    // FULL is deliberate: once the desktop reports a local edit as committed,
    // a host crash must not trade durability for a small latency improvement.
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        PRAGMA wal_autocheckpoint = 1000;
        "#,
    )?;
    Ok(())
}

fn validate_edit(edit: &CommitSkillEdit) -> Result<(), LocalStoreError> {
    for (field, value) in [
        ("skill_id", edit.skill_id.as_str()),
        ("name", edit.name.as_str()),
        ("slug", edit.slug.as_str()),
        ("blob_hash", edit.blob_hash.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(LocalStoreError::InvalidInput(format!(
                "{field} must not be empty"
            )));
        }
    }
    if edit.workspace_path.as_os_str().is_empty() {
        return Err(LocalStoreError::InvalidInput(
            "workspace_path must not be empty".to_owned(),
        ));
    }
    if edit.base_revision.is_some_and(|revision| revision < 0) {
        return Err(LocalStoreError::InvalidInput(
            "base_revision must not be negative".to_owned(),
        ));
    }
    Ok(())
}

fn path_to_string(path: &Path) -> Result<String, LocalStoreError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| LocalStoreError::NonUtf8Path(path.to_path_buf()))
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
    #[error("path is not valid UTF-8: {0:?}")]
    NonUtf8Path(PathBuf),
}

#[cfg(test)]
mod tests {
    use super::*;

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
            workspace_path: PathBuf::from("/tmp/skillhive/code-review"),
            blob_hash: "sha256:abc123".to_owned(),
            base_revision: None,
            operation: MutationOperation::Create,
        }
    }

    #[test]
    fn initializes_schema_and_pragmas() {
        let (_temp, store) = open_temp_store();
        let health = store.health().expect("health");
        assert_eq!(health.schema_version, migrations::LATEST_SCHEMA_VERSION);
        assert_eq!(health.pending_mutations, 0);

        let connection = store.lock_connection().expect("connection");
        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("foreign_keys");
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn local_edit_and_outbox_entry_commit_together() {
        let (_temp, store) = open_temp_store();
        let mutation = store.commit_skill_edit(sample_edit("skill-1")).expect("commit");

        assert_eq!(mutation.state, MutationState::Pending);
        let skill = store.get_skill("skill-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Dirty);

        let pending = store.list_dispatchable_mutations(10).expect("mutations");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, mutation.id);
        assert_eq!(pending[0].skill_id, "skill-1");
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
    fn delete_is_queued_without_dropping_local_record() {
        let (_temp, store) = open_temp_store();
        store.commit_skill_edit(sample_edit("skill-1")).expect("commit");
        let deletion = store.queue_skill_delete("skill-1", None).expect("delete");

        assert_eq!(deletion.operation, MutationOperation::Delete);
        assert!(store.get_skill("skill-1").expect("read").is_some());
        assert_eq!(store.list_dispatchable_mutations(10).expect("mutations").len(), 2);
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
}
