use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    blob_store::BlobStore,
    skill_snapshot::{materialize_snapshot, SnapshotError},
    snapshot_verifier::{verify_materialized_snapshot, SnapshotVerificationError},
};

const RESERVED_PREFIX: &str = ".skillhive-";

#[derive(Debug, Clone)]
pub struct DeploymentEngine {
    journal_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DeploymentRequest {
    pub skill_id: String,
    pub agent_profile_id: String,
    pub snapshot_hash: String,
    pub skill_root: PathBuf,
    pub directory_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentResult {
    pub transaction_id: String,
    pub skill_id: String,
    pub agent_profile_id: String,
    pub target_path: PathBuf,
    pub snapshot_hash: String,
    pub replaced_existing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeploymentPhase {
    Intent,
    Prepared,
    OldMoved,
    Activated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeploymentJournal {
    transaction_id: String,
    skill_id: String,
    agent_profile_id: String,
    snapshot_hash: String,
    skill_root: PathBuf,
    destination: PathBuf,
    staging: PathBuf,
    backup: PathBuf,
    replaced_existing: bool,
    phase: DeploymentPhase,
}

impl DeploymentJournal {
    fn result(&self) -> DeploymentResult {
        DeploymentResult {
            transaction_id: self.transaction_id.clone(),
            skill_id: self.skill_id.clone(),
            agent_profile_id: self.agent_profile_id.clone(),
            target_path: self.destination.clone(),
            snapshot_hash: self.snapshot_hash.clone(),
            replaced_existing: self.replaced_existing,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub recovered: usize,
    pub rolled_back: usize,
    pub catalog_commits: Vec<DeploymentResult>,
    pub failed: Vec<String>,
}

impl DeploymentEngine {
    pub fn open(journal_root: impl AsRef<Path>) -> Result<Self, DeploymentError> {
        let journal_root = journal_root.as_ref().to_path_buf();
        ensure_real_directory(&journal_root)?;
        Ok(Self { journal_root })
    }

    pub fn journal_root(&self) -> &Path {
        &self.journal_root
    }

    /// Materializes an immutable snapshot, atomically activates it, and leaves
    /// a durable journal until the matching SQLite catalog row is committed.
    pub fn deploy(
        &self,
        blobs: &BlobStore,
        request: DeploymentRequest,
    ) -> Result<DeploymentResult, DeploymentError> {
        validate_request(&request)?;
        ensure_directory_root(&request.skill_root)?;

        let transaction_id = Uuid::new_v4().to_string();
        let destination = request.skill_root.join(&request.directory_name);
        let staging = request
            .skill_root
            .join(format!("{RESERVED_PREFIX}stage-{transaction_id}"));
        let backup = request
            .skill_root
            .join(format!("{RESERVED_PREFIX}backup-{transaction_id}"));
        let journal_path = self.journal_path(&transaction_id);
        let replaced_existing = path_entry_exists(&destination)?;

        let mut journal = DeploymentJournal {
            transaction_id: transaction_id.clone(),
            skill_id: request.skill_id.clone(),
            agent_profile_id: request.agent_profile_id.clone(),
            snapshot_hash: request.snapshot_hash.clone(),
            skill_root: request.skill_root.clone(),
            destination: destination.clone(),
            staging: staging.clone(),
            backup: backup.clone(),
            replaced_existing,
            phase: DeploymentPhase::Intent,
        };
        self.persist_journal(&journal_path, &journal)?;

        let staging_result = (|| -> Result<(), DeploymentError> {
            materialize_snapshot(blobs, &request.snapshot_hash, &staging)?;
            verify_materialized_snapshot(blobs, &request.snapshot_hash, &staging)?;
            sync_directory(&request.skill_root)?;
            journal.phase = DeploymentPhase::Prepared;
            self.persist_journal(&journal_path, &journal)?;
            Ok(())
        })();
        if let Err(error) = staging_result {
            if path_entry_exists(&staging).unwrap_or(false) {
                remove_any(&staging).ok();
                sync_directory(&request.skill_root).ok();
            }
            self.remove_journal(&journal_path).ok();
            return Err(error);
        }

        if replaced_existing {
            fs::rename(&destination, &backup)?;
            sync_directory(&request.skill_root)?;
            journal.phase = DeploymentPhase::OldMoved;
            self.persist_journal(&journal_path, &journal)?;
        }

        if let Err(error) = fs::rename(&staging, &destination) {
            return Err(DeploymentError::Io(error));
        }
        sync_directory(&request.skill_root)?;

        // Do not destroy the old known-good target until the newly activated
        // directory proves it is exactly the immutable snapshot.
        if let Err(error) = verify_materialized_snapshot(
            blobs,
            &request.snapshot_hash,
            &destination,
        ) {
            if replaced_existing && path_entry_exists(&backup)? {
                rollback_active_to_backup(&journal)?;
            } else if path_entry_exists(&destination)? {
                remove_any(&destination)?;
                sync_directory(&request.skill_root)?;
            }
            self.remove_journal(&journal_path).ok();
            return Err(DeploymentError::Verification(error));
        }

        journal.phase = DeploymentPhase::Activated;
        self.persist_journal(&journal_path, &journal)?;
        if path_entry_exists(&backup)? {
            remove_any(&backup)?;
            sync_directory(&request.skill_root)?;
        }

        Ok(journal.result())
    }

    /// ACKs the journal only after re-verifying the active directory. This
    /// narrows the verification -> SQLite commit -> journal cleanup tamper window.
    pub fn acknowledge_catalog_commit(
        &self,
        blobs: &BlobStore,
        transaction_id: &str,
    ) -> Result<(), DeploymentError> {
        validate_transaction_id(transaction_id)?;
        let journal_path = self.journal_path(transaction_id);
        let journal = read_latest_journal(&journal_path)?;
        validate_journal_paths(&journal)?;
        if journal.transaction_id != transaction_id || journal.phase != DeploymentPhase::Activated {
            return Err(DeploymentError::CatalogAckRejected(transaction_id.to_owned()));
        }
        if !path_is_real_directory(&journal.destination)?
            || path_entry_exists(&journal.staging)?
            || path_entry_exists(&journal.backup)?
        {
            return Err(DeploymentError::CatalogAckRejected(transaction_id.to_owned()));
        }
        verify_materialized_snapshot(blobs, &journal.snapshot_hash, &journal.destination)?;
        self.remove_journal(&journal_path)
    }

    /// Recovers interrupted deployment transactions. Roll-forward is permitted
    /// only after the candidate snapshot is cryptographically verified. If a
    /// known-good backup exists and the candidate cannot be proven valid, the
    /// recovery path restores the backup instead.
    pub fn recover_incomplete(
        &self,
        blobs: &BlobStore,
    ) -> Result<RecoveryReport, DeploymentError> {
        ensure_real_directory(&self.journal_root)?;
        let mut report = RecoveryReport {
            recovered: 0,
            rolled_back: 0,
            catalog_commits: Vec::new(),
            failed: Vec::new(),
        };

        let mut journals = fs::read_dir(&self.journal_root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("journal"))
            .collect::<Vec<_>>();
        journals.sort();

        for journal_path in journals {
            match self.recover_one(blobs, &journal_path) {
                Ok(RecoveryOutcome::Committed(result)) => {
                    report.recovered += 1;
                    report.catalog_commits.push(result);
                }
                Ok(RecoveryOutcome::RolledBack) => report.rolled_back += 1,
                Err(error) => report
                    .failed
                    .push(format!("{}: {error}", journal_path.display())),
            }
        }

        Ok(report)
    }

    fn recover_one(
        &self,
        blobs: &BlobStore,
        journal_path: &Path,
    ) -> Result<RecoveryOutcome, DeploymentError> {
        let mut journal = read_latest_journal(journal_path)?;
        validate_journal_paths(&journal)?;
        ensure_directory_root(&journal.skill_root)?;

        let destination_exists = path_entry_exists(&journal.destination)?;
        let staging_exists = path_entry_exists(&journal.staging)?;
        let backup_exists = path_entry_exists(&journal.backup)?;

        match journal.phase {
            DeploymentPhase::Intent => {
                if backup_exists {
                    return Err(unrecoverable(&journal, "backup exists while phase is intent"));
                }
                if staging_exists {
                    remove_any(&journal.staging)?;
                    sync_directory(&journal.skill_root)?;
                }
                self.remove_journal(journal_path)?;
                return Ok(RecoveryOutcome::RolledBack);
            }
            DeploymentPhase::Prepared => {
                if journal.replaced_existing {
                    if destination_exists && staging_exists && !backup_exists {
                        // Old active target was never moved. Discard staging.
                        remove_any(&journal.staging)?;
                        sync_directory(&journal.skill_root)?;
                        self.remove_journal(journal_path)?;
                        return Ok(RecoveryOutcome::RolledBack);
                    }
                    if !destination_exists && staging_exists && backup_exists {
                        return self.activate_verified_staging(blobs, journal_path, &mut journal);
                    }
                    if destination_exists && !staging_exists && backup_exists {
                        return self.verify_active_or_restore_backup(
                            blobs,
                            journal_path,
                            &mut journal,
                        );
                    }
                    if !destination_exists && !staging_exists && backup_exists {
                        restore_backup(&journal)?;
                        self.remove_journal(journal_path)?;
                        return Ok(RecoveryOutcome::RolledBack);
                    }
                } else {
                    if !destination_exists && staging_exists && !backup_exists {
                        // Activation was not started; conservative rollback.
                        remove_any(&journal.staging)?;
                        sync_directory(&journal.skill_root)?;
                        self.remove_journal(journal_path)?;
                        return Ok(RecoveryOutcome::RolledBack);
                    }
                    if destination_exists && !staging_exists && !backup_exists {
                        return self.verify_active_without_backup(
                            blobs,
                            journal_path,
                            &mut journal,
                        );
                    }
                }
            }
            DeploymentPhase::OldMoved => {
                if !destination_exists && staging_exists && backup_exists {
                    return self.activate_verified_staging(blobs, journal_path, &mut journal);
                }
                if destination_exists && !staging_exists && backup_exists {
                    return self.verify_active_or_restore_backup(
                        blobs,
                        journal_path,
                        &mut journal,
                    );
                }
                if !destination_exists && !staging_exists && backup_exists {
                    restore_backup(&journal)?;
                    self.remove_journal(journal_path)?;
                    return Ok(RecoveryOutcome::RolledBack);
                }
            }
            DeploymentPhase::Activated => {
                if !destination_exists {
                    if backup_exists {
                        restore_backup(&journal)?;
                        self.remove_journal(journal_path)?;
                        return Ok(RecoveryOutcome::RolledBack);
                    }
                    return Err(unrecoverable(
                        &journal,
                        "activated target is missing and no backup remains",
                    ));
                }
                if staging_exists {
                    return Err(unrecoverable(
                        &journal,
                        "staging still exists after activation",
                    ));
                }
                return self.verify_active_or_restore_backup(
                    blobs,
                    journal_path,
                    &mut journal,
                );
            }
        }

        Err(unrecoverable(
            &journal,
            &format!(
                "destination={destination_exists}, staging={staging_exists}, backup={backup_exists}, phase={:?}",
                journal.phase
            ),
        ))
    }

    fn activate_verified_staging(
        &self,
        blobs: &BlobStore,
        journal_path: &Path,
        journal: &mut DeploymentJournal,
    ) -> Result<RecoveryOutcome, DeploymentError> {
        if let Err(error) = verify_materialized_snapshot(
            blobs,
            &journal.snapshot_hash,
            &journal.staging,
        ) {
            if path_entry_exists(&journal.backup)? {
                if path_entry_exists(&journal.staging)? {
                    remove_any(&journal.staging)?;
                }
                restore_backup(journal)?;
                self.remove_journal(journal_path)?;
                return Ok(RecoveryOutcome::RolledBack);
            }
            return Err(DeploymentError::Verification(error));
        }

        fs::rename(&journal.staging, &journal.destination)?;
        sync_directory(&journal.skill_root)?;
        verify_materialized_snapshot(blobs, &journal.snapshot_hash, &journal.destination)?;
        journal.phase = DeploymentPhase::Activated;
        self.persist_journal(journal_path, journal)?;
        if path_entry_exists(&journal.backup)? {
            remove_any(&journal.backup)?;
            sync_directory(&journal.skill_root)?;
        }
        Ok(RecoveryOutcome::Committed(journal.result()))
    }

    fn verify_active_or_restore_backup(
        &self,
        blobs: &BlobStore,
        journal_path: &Path,
        journal: &mut DeploymentJournal,
    ) -> Result<RecoveryOutcome, DeploymentError> {
        match verify_materialized_snapshot(blobs, &journal.snapshot_hash, &journal.destination) {
            Ok(_) => {
                if path_entry_exists(&journal.staging)? {
                    remove_any(&journal.staging)?;
                }
                journal.phase = DeploymentPhase::Activated;
                self.persist_journal(journal_path, journal)?;
                if path_entry_exists(&journal.backup)? {
                    remove_any(&journal.backup)?;
                    sync_directory(&journal.skill_root)?;
                }
                Ok(RecoveryOutcome::Committed(journal.result()))
            }
            Err(error) if path_entry_exists(&journal.backup)? => {
                rollback_active_to_backup(journal)?;
                self.remove_journal(journal_path)?;
                Ok(RecoveryOutcome::RolledBack)
            }
            Err(error) => Err(DeploymentError::Verification(error)),
        }
    }

    fn verify_active_without_backup(
        &self,
        blobs: &BlobStore,
        journal_path: &Path,
        journal: &mut DeploymentJournal,
    ) -> Result<RecoveryOutcome, DeploymentError> {
        match verify_materialized_snapshot(blobs, &journal.snapshot_hash, &journal.destination) {
            Ok(_) => {
                journal.phase = DeploymentPhase::Activated;
                self.persist_journal(journal_path, journal)?;
                Ok(RecoveryOutcome::Committed(journal.result()))
            }
            Err(error) => {
                // This was a new install. There is no previous known-good target
                // to preserve, so remove only the failed candidate and roll back.
                if path_entry_exists(&journal.destination)? {
                    remove_any(&journal.destination)?;
                    sync_directory(&journal.skill_root)?;
                }
                self.remove_journal(journal_path)?;
                let _ = error;
                Ok(RecoveryOutcome::RolledBack)
            }
        }
    }

    fn journal_path(&self, transaction_id: &str) -> PathBuf {
        self.journal_root.join(format!("{transaction_id}.journal"))
    }

    fn persist_journal(
        &self,
        journal_path: &Path,
        journal: &DeploymentJournal,
    ) -> Result<(), DeploymentError> {
        let existed = path_entry_exists(journal_path)?;
        if existed {
            let metadata = fs::symlink_metadata(journal_path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(DeploymentError::InvalidJournal(format!(
                    "journal path is not a regular file: {}",
                    journal_path.display()
                )));
            }
        }
        let bytes = serde_json::to_vec(journal)?;
        let mut file = OpenOptions::new()
            .write(true)
            .append(true)
            .create(true)
            .open(journal_path)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        if !existed {
            sync_directory(&self.journal_root)?;
        }
        Ok(())
    }

    fn remove_journal(&self, journal_path: &Path) -> Result<(), DeploymentError> {
        if path_entry_exists(journal_path)? {
            let metadata = fs::symlink_metadata(journal_path)?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(DeploymentError::InvalidJournal(format!(
                    "journal path is not a regular file: {}",
                    journal_path.display()
                )));
            }
            fs::remove_file(journal_path)?;
            sync_directory(&self.journal_root)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RecoveryOutcome {
    Committed(DeploymentResult),
    RolledBack,
}

fn read_latest_journal(path: &Path) -> Result<DeploymentJournal, DeploymentError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(DeploymentError::InvalidJournal(format!(
            "journal path is not a regular file: {}",
            path.display()
        )));
    }
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut latest = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<DeploymentJournal>(&line) {
            Ok(record) => latest = Some(record),
            Err(_) if latest.is_some() => break,
            Err(error) => return Err(DeploymentError::JournalSerialization(error)),
        }
    }

    latest.ok_or_else(|| {
        DeploymentError::InvalidJournal(format!(
            "{} contains no complete journal record",
            path.display()
        ))
    })
}

fn validate_request(request: &DeploymentRequest) -> Result<(), DeploymentError> {
    for (field, value) in [
        ("skill_id", request.skill_id.as_str()),
        ("agent_profile_id", request.agent_profile_id.as_str()),
        ("snapshot_hash", request.snapshot_hash.as_str()),
        ("directory_name", request.directory_name.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(DeploymentError::InvalidRequest(format!(
                "{field} must not be empty"
            )));
        }
    }

    validate_directory_name(&request.directory_name)?;
    if !request.skill_root.is_absolute() || request.skill_root.to_str().is_none() {
        return Err(DeploymentError::InvalidRequest(
            "skill_root must be an absolute UTF-8 path".to_owned(),
        ));
    }
    Ok(())
}

fn validate_transaction_id(value: &str) -> Result<(), DeploymentError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(DeploymentError::InvalidTransactionId(value.to_owned()));
    }
    Ok(())
}

fn validate_directory_name(name: &str) -> Result<(), DeploymentError> {
    if name.starts_with(RESERVED_PREFIX)
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|character| {
            character.is_control()
                || matches!(character, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*')
        })
    {
        return Err(DeploymentError::InvalidDirectoryName(name.to_owned()));
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) if !component.is_empty() => {}
        _ => return Err(DeploymentError::InvalidDirectoryName(name.to_owned())),
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        || stem
            .strip_prefix("LPT")
            .is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    if reserved {
        return Err(DeploymentError::InvalidDirectoryName(name.to_owned()));
    }
    Ok(())
}

fn ensure_directory_root(path: &Path) -> Result<(), DeploymentError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(DeploymentError::InvalidSkillRoot(path.to_path_buf()));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(DeploymentError::InvalidSkillRoot(path.to_path_buf()));
            }
            sync_existing_ancestor(path)?;
        }
        Err(error) => return Err(DeploymentError::Io(error)),
    }
    Ok(())
}

fn ensure_real_directory(path: &Path) -> Result<(), DeploymentError> {
    ensure_directory_root(path)
}

fn path_entry_exists(path: &Path) -> Result<bool, DeploymentError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(DeploymentError::Io(error)),
    }
}

fn path_is_real_directory(path: &Path) -> Result<bool, DeploymentError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.is_dir() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(DeploymentError::Io(error)),
    }
}

fn validate_journal_paths(journal: &DeploymentJournal) -> Result<(), DeploymentError> {
    if !journal.skill_root.is_absolute() {
        return Err(DeploymentError::InvalidJournal(format!(
            "transaction {} contains a relative skill root",
            journal.transaction_id
        )));
    }
    for path in [&journal.destination, &journal.staging, &journal.backup] {
        if path.parent() != Some(journal.skill_root.as_path()) {
            return Err(DeploymentError::InvalidJournal(format!(
                "transaction {} contains a path outside its skill root",
                journal.transaction_id
            )));
        }
    }
    Ok(())
}

fn rollback_active_to_backup(journal: &DeploymentJournal) -> Result<(), DeploymentError> {
    if path_entry_exists(&journal.destination)? {
        remove_any(&journal.destination)?;
    }
    restore_backup(journal)
}

fn restore_backup(journal: &DeploymentJournal) -> Result<(), DeploymentError> {
    if path_entry_exists(&journal.destination)? {
        return Err(unrecoverable(
            journal,
            "cannot restore backup while destination exists",
        ));
    }
    if !path_entry_exists(&journal.backup)? {
        return Err(unrecoverable(journal, "backup is missing"));
    }
    fs::rename(&journal.backup, &journal.destination)?;
    sync_directory(&journal.skill_root)?;
    Ok(())
}

fn remove_any(path: &Path) -> Result<(), DeploymentError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
        remove_tree_without_following_symlinks(path)?;
    } else {
        return Err(DeploymentError::UnsupportedFileType(path.to_path_buf()));
    }
    Ok(())
}

fn remove_tree_without_following_symlinks(path: &Path) -> Result<(), DeploymentError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(DeploymentError::UnsupportedFileType(path.to_path_buf()));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        remove_tree_without_following_symlinks(&entry.path())?;
    }
    fs::remove_dir(path)?;
    Ok(())
}

fn unrecoverable(journal: &DeploymentJournal, reason: &str) -> DeploymentError {
    DeploymentError::UnrecoverableTransaction {
        transaction_id: journal.transaction_id.clone(),
        reason: reason.to_owned(),
    }
}

fn sync_existing_ancestor(path: &Path) -> Result<(), DeploymentError> {
    if let Some(parent) = path.parent() {
        if path_is_real_directory(parent)? {
            sync_directory(parent)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), DeploymentError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), DeploymentError> {
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DeploymentError {
    #[error("deployment filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("skill snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("deployed snapshot verification error: {0}")]
    Verification(#[from] SnapshotVerificationError),
    #[error("deployment journal serialization error: {0}")]
    JournalSerialization(#[from] serde_json::Error),
    #[error("invalid deployment request: {0}")]
    InvalidRequest(String),
    #[error("invalid deployment transaction id: {0}")]
    InvalidTransactionId(String),
    #[error("deployment catalog ACK rejected for transaction {0}")]
    CatalogAckRejected(String),
    #[error("invalid skill directory name: {0}")]
    InvalidDirectoryName(String),
    #[error("invalid agent skill root: {0:?}")]
    InvalidSkillRoot(PathBuf),
    #[error("unsupported file type in deployment path: {0:?}")]
    UnsupportedFileType(PathBuf),
    #[error("invalid deployment journal: {0}")]
    InvalidJournal(String),
    #[error("deployment transaction {transaction_id} cannot be recovered: {reason}")]
    UnrecoverableTransaction {
        transaction_id: String,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_snapshot::{capture_workspace, SnapshotPolicy};

    fn make_workspace(root: &Path, marker: &str) {
        fs::create_dir_all(root.join("scripts")).expect("mkdir");
        fs::write(root.join("SKILL.md"), format!("# {marker}\n")).expect("skill");
        fs::write(root.join("scripts").join("run.py"), marker).expect("script");
    }

    fn request(snapshot_hash: String, skill_root: PathBuf) -> DeploymentRequest {
        DeploymentRequest {
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "claude-code:default".to_owned(),
            snapshot_hash,
            skill_root,
            directory_name: "code-review".to_owned(),
        }
    }

    #[test]
    fn successful_deploy_requires_verified_catalog_ack() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        make_workspace(&workspace, "v1");
        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");
        let snapshot = capture_workspace(&blobs, &workspace, SnapshotPolicy::default())
            .expect("snapshot");
        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");

        let result = engine
            .deploy(
                &blobs,
                request(snapshot.manifest_hash, temp.path().join("agent-skills")),
            )
            .expect("deploy");
        engine
            .acknowledge_catalog_commit(&blobs, &result.transaction_id)
            .expect("ack");
    }

    #[test]
    fn prepared_new_install_rolls_back_on_recovery() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        make_workspace(&workspace, "v1");
        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");
        let snapshot = capture_workspace(&blobs, &workspace, SnapshotPolicy::default())
            .expect("snapshot");
        let root = temp.path().join("skills");
        fs::create_dir_all(&root).expect("root");
        let staging = root.join(".skillhive-stage-tx1");
        materialize_snapshot(&blobs, &snapshot.manifest_hash, &staging).expect("stage");
        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");
        let journal = DeploymentJournal {
            transaction_id: "tx1".to_owned(),
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "test".to_owned(),
            snapshot_hash: snapshot.manifest_hash,
            skill_root: root.clone(),
            destination: root.join("demo"),
            staging,
            backup: root.join(".skillhive-backup-tx1"),
            replaced_existing: false,
            phase: DeploymentPhase::Prepared,
        };
        engine
            .persist_journal(&engine.journal_path("tx1"), &journal)
            .expect("journal");

        let report = engine.recover_incomplete(&blobs).expect("recover");
        assert_eq!(report.rolled_back, 1);
        assert!(report.catalog_commits.is_empty());
    }
}
