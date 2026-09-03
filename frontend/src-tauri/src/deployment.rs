use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SKILL_ENTRYPOINT: &str = "SKILL.md";
const RESERVED_PREFIX: &str = ".skillhive-";

/// Filesystem deployment engine for already-authorized local skill material.
///
/// Authorization is intentionally resolved before this layer. The engine owns
/// filesystem durability only. A successful deployment journal remains until
/// the caller commits the matching SQLite catalog row and explicitly ACKs it.
#[derive(Debug, Clone)]
pub struct DeploymentEngine {
    journal_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DeploymentRequest {
    pub skill_id: String,
    pub agent_profile_id: String,
    pub source_dir: PathBuf,
    pub skill_root: PathBuf,
    pub directory_name: String,
    pub blob_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentResult {
    pub transaction_id: String,
    pub skill_id: String,
    pub agent_profile_id: String,
    pub target_path: PathBuf,
    pub blob_hash: String,
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
    blob_hash: String,
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
            blob_hash: self.blob_hash.clone(),
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
        fs::create_dir_all(&journal_root)?;
        Ok(Self { journal_root })
    }

    pub fn journal_root(&self) -> &Path {
        &self.journal_root
    }

    /// Installs or updates one skill directory.
    ///
    /// The returned deployment is filesystem-complete but intentionally not
    /// finalized. The caller must durably record it in LocalStore and then call
    /// `acknowledge_catalog_commit` with the transaction id.
    pub fn deploy(&self, request: DeploymentRequest) -> Result<DeploymentResult, DeploymentError> {
        validate_request(&request)?;
        validate_source_skill(&request.source_dir)?;
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
        let replaced_existing = destination.exists();

        let mut journal = DeploymentJournal {
            transaction_id: transaction_id.clone(),
            skill_id: request.skill_id.clone(),
            agent_profile_id: request.agent_profile_id.clone(),
            blob_hash: request.blob_hash.clone(),
            skill_root: request.skill_root.clone(),
            destination: destination.clone(),
            staging: staging.clone(),
            backup: backup.clone(),
            replaced_existing,
            phase: DeploymentPhase::Intent,
        };
        self.persist_journal(&journal_path, &journal)?;

        let staging_result = (|| -> Result<(), DeploymentError> {
            copy_tree_durable(&request.source_dir, &staging)?;
            validate_source_skill(&staging)?;
            sync_directory(&request.skill_root)?;
            journal.phase = DeploymentPhase::Prepared;
            self.persist_journal(&journal_path, &journal)?;
            Ok(())
        })();
        if let Err(error) = staging_result {
            if staging.exists() {
                remove_any(&staging).ok();
                sync_directory(&request.skill_root).ok();
            }
            self.remove_journal(&journal_path).ok();
            return Err(error);
        }

        let swap_result = (|| -> Result<(), DeploymentError> {
            if replaced_existing {
                fs::rename(&destination, &backup)?;
                sync_directory(&request.skill_root)?;
                journal.phase = DeploymentPhase::OldMoved;
                self.persist_journal(&journal_path, &journal)?;
            }

            fs::rename(&staging, &destination)?;
            sync_directory(&request.skill_root)?;
            journal.phase = DeploymentPhase::Activated;
            self.persist_journal(&journal_path, &journal)?;

            if backup.exists() {
                remove_any(&backup)?;
                sync_directory(&request.skill_root)?;
            }
            Ok(())
        })();

        if let Err(error) = swap_result {
            return Err(error);
        }

        Ok(journal.result())
    }

    /// Removes an Activated journal only after the caller has durably committed
    /// the matching deployment row to SQLite.
    pub fn acknowledge_catalog_commit(&self, transaction_id: &str) -> Result<(), DeploymentError> {
        validate_transaction_id(transaction_id)?;
        let journal_path = self.journal_path(transaction_id);
        let journal = read_latest_journal(&journal_path)?;
        validate_journal_paths(&journal)?;
        if journal.transaction_id != transaction_id || journal.phase != DeploymentPhase::Activated {
            return Err(DeploymentError::CatalogAckRejected(transaction_id.to_owned()));
        }
        if !journal.destination.is_dir() || journal.staging.exists() || journal.backup.exists() {
            return Err(DeploymentError::CatalogAckRejected(transaction_id.to_owned()));
        }
        self.remove_journal(&journal_path)
    }

    /// Reconciles interrupted filesystem transactions before new deployments.
    /// Activated results are returned in `catalog_commits` and their journals
    /// remain until SQLite persistence is ACKed by the caller.
    pub fn recover_incomplete(&self) -> Result<RecoveryReport, DeploymentError> {
        fs::create_dir_all(&self.journal_root)?;
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
            match self.recover_one(&journal_path) {
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

    fn recover_one(&self, journal_path: &Path) -> Result<RecoveryOutcome, DeploymentError> {
        let mut journal = read_latest_journal(journal_path)?;
        validate_journal_paths(&journal)?;

        let destination_exists = journal.destination.exists();
        let staging_exists = journal.staging.exists();
        let backup_exists = journal.backup.exists();

        if journal.phase == DeploymentPhase::Intent {
            if backup_exists {
                return Err(DeploymentError::UnrecoverableTransaction {
                    transaction_id: journal.transaction_id,
                    reason: "backup exists while latest durable phase is intent".to_owned(),
                });
            }
            if staging_exists {
                remove_any(&journal.staging)?;
                sync_directory(&journal.skill_root)?;
            }
            self.remove_journal(journal_path)?;
            return Ok(RecoveryOutcome::RolledBack);
        }

        if destination_exists {
            if journal.phase == DeploymentPhase::Prepared
                && journal.replaced_existing
                && !backup_exists
            {
                if staging_exists {
                    remove_any(&journal.staging)?;
                    sync_directory(&journal.skill_root)?;
                }
                self.remove_journal(journal_path)?;
                return Ok(RecoveryOutcome::RolledBack);
            }

            if journal.phase == DeploymentPhase::Prepared
                && !journal.replaced_existing
                && staging_exists
            {
                return Err(DeploymentError::UnrecoverableTransaction {
                    transaction_id: journal.transaction_id,
                    reason: "new destination and staging both exist before activation was recorded"
                        .to_owned(),
                });
            }

            if journal.phase == DeploymentPhase::OldMoved && staging_exists && backup_exists {
                return Err(DeploymentError::UnrecoverableTransaction {
                    transaction_id: journal.transaction_id,
                    reason: "active, staging, and backup paths all exist after old target was moved"
                        .to_owned(),
                });
            }

            if staging_exists {
                remove_any(&journal.staging)?;
            }
            if backup_exists {
                remove_any(&journal.backup)?;
            }
            sync_directory(&journal.skill_root)?;
            if journal.phase != DeploymentPhase::Activated {
                journal.phase = DeploymentPhase::Activated;
                self.persist_journal(journal_path, &journal)?;
            }
            return Ok(RecoveryOutcome::Committed(journal.result()));
        }

        if staging_exists && backup_exists {
            fs::rename(&journal.staging, &journal.destination)?;
            sync_directory(&journal.skill_root)?;
            remove_any(&journal.backup)?;
            sync_directory(&journal.skill_root)?;
            journal.phase = DeploymentPhase::Activated;
            self.persist_journal(journal_path, &journal)?;
            return Ok(RecoveryOutcome::Committed(journal.result()));
        }

        if !staging_exists && backup_exists {
            fs::rename(&journal.backup, &journal.destination)?;
            sync_directory(&journal.skill_root)?;
            self.remove_journal(journal_path)?;
            return Ok(RecoveryOutcome::RolledBack);
        }

        if staging_exists && !backup_exists && journal.phase == DeploymentPhase::Prepared {
            remove_any(&journal.staging)?;
            sync_directory(&journal.skill_root)?;
            self.remove_journal(journal_path)?;
            return Ok(RecoveryOutcome::RolledBack);
        }

        Err(DeploymentError::UnrecoverableTransaction {
            transaction_id: journal.transaction_id,
            reason: format!(
                "destination={}, staging={}, backup={}, phase={:?}",
                destination_exists, staging_exists, backup_exists, journal.phase
            ),
        })
    }

    fn journal_path(&self, transaction_id: &str) -> PathBuf {
        self.journal_root.join(format!("{transaction_id}.journal"))
    }

    fn persist_journal(
        &self,
        journal_path: &Path,
        journal: &DeploymentJournal,
    ) -> Result<(), DeploymentError> {
        let existed = journal_path.exists();
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
        if journal_path.exists() {
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
        ("blob_hash", request.blob_hash.as_str()),
        ("directory_name", request.directory_name.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(DeploymentError::InvalidRequest(format!(
                "{field} must not be empty"
            )));
        }
    }

    validate_directory_name(&request.directory_name)?;
    for (field, path) in [
        ("source_dir", request.source_dir.as_path()),
        ("skill_root", request.skill_root.as_path()),
    ] {
        if !path.is_absolute() {
            return Err(DeploymentError::InvalidRequest(format!(
                "{field} must be an absolute path"
            )));
        }
        if path.to_str().is_none() {
            return Err(DeploymentError::InvalidRequest(format!(
                "{field} must be valid UTF-8"
            )));
        }
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
    if name.starts_with(RESERVED_PREFIX) {
        return Err(DeploymentError::InvalidDirectoryName(name.to_owned()));
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(component)), None) if !component.is_empty() => Ok(()),
        _ => Err(DeploymentError::InvalidDirectoryName(name.to_owned())),
    }
}

fn ensure_directory_root(path: &Path) -> Result<(), DeploymentError> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(DeploymentError::InvalidSkillRoot(path.to_path_buf()));
        }
    } else {
        fs::create_dir_all(path)?;
        sync_existing_ancestor(path)?;
    }
    Ok(())
}

fn validate_source_skill(path: &Path) -> Result<(), DeploymentError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| DeploymentError::InvalidSourceSkill(path.to_path_buf()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(DeploymentError::InvalidSourceSkill(path.to_path_buf()));
    }

    let entrypoint = path.join(SKILL_ENTRYPOINT);
    let entrypoint_metadata = fs::symlink_metadata(&entrypoint)
        .map_err(|_| DeploymentError::MissingSkillEntrypoint(entrypoint.clone()))?;
    if entrypoint_metadata.file_type().is_symlink() || !entrypoint_metadata.is_file() {
        return Err(DeploymentError::MissingSkillEntrypoint(entrypoint));
    }
    Ok(())
}

fn copy_tree_durable(source: &Path, destination: &Path) -> Result<(), DeploymentError> {
    if destination.exists() {
        return Err(DeploymentError::StagingCollision(destination.to_path_buf()));
    }
    fs::create_dir(destination)?;

    let mut entries = fs::read_dir(source)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let source_path = entry.path();
        let target_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        let file_type = metadata.file_type();

        if file_type.is_symlink() {
            return Err(DeploymentError::SymlinkNotAllowed(source_path));
        }
        if file_type.is_dir() {
            copy_tree_durable(&source_path, &target_path)?;
        } else if file_type.is_file() {
            let mut source_file = File::open(&source_path)?;
            let mut target_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target_path)?;
            io::copy(&mut source_file, &mut target_file)?;
            target_file.sync_all()?;
        } else {
            return Err(DeploymentError::UnsupportedFileType(source_path));
        }
    }

    sync_directory(destination)?;
    Ok(())
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

fn remove_any(path: &Path) -> Result<(), DeploymentError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        return Err(DeploymentError::UnsupportedFileType(path.to_path_buf()));
    }
    Ok(())
}

fn sync_existing_ancestor(path: &Path) -> Result<(), DeploymentError> {
    if let Some(parent) = path.parent() {
        if parent.exists() {
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
    #[error("invalid agent skill root: {0}")]
    InvalidSkillRoot(PathBuf),
    #[error("invalid source skill directory: {0}")]
    InvalidSourceSkill(PathBuf),
    #[error("skill entrypoint is missing or invalid: {0}")]
    MissingSkillEntrypoint(PathBuf),
    #[error("symbolic links are not allowed in managed skills: {0}")]
    SymlinkNotAllowed(PathBuf),
    #[error("unsupported file type in skill tree: {0}")]
    UnsupportedFileType(PathBuf),
    #[error("staging path unexpectedly exists: {0}")]
    StagingCollision(PathBuf),
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

    fn make_skill(root: &Path, marker: &str) {
        fs::create_dir_all(root.join("scripts")).expect("mkdir");
        fs::write(root.join(SKILL_ENTRYPOINT), format!("# {marker}\n")).expect("skill md");
        fs::write(root.join("scripts").join("run.sh"), marker).expect("script");
    }

    fn request(source: PathBuf, skill_root: PathBuf) -> DeploymentRequest {
        DeploymentRequest {
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "claude-code:default".to_owned(),
            source_dir: source,
            skill_root,
            directory_name: "code-review".to_owned(),
            blob_hash: "sha256:abc".to_owned(),
        }
    }

    #[test]
    fn successful_deploy_requires_catalog_ack() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let skill_root = temp.path().join("agent-skills");
        make_skill(&source, "v1");
        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");

        let result = engine.deploy(request(source, skill_root)).expect("deploy");
        assert!(engine.journal_path(&result.transaction_id).exists());
        engine
            .acknowledge_catalog_commit(&result.transaction_id)
            .expect("ack");
        assert!(!engine.journal_path(&result.transaction_id).exists());
    }

    #[test]
    fn recovery_rolls_forward_and_requests_catalog_commit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill_root = temp.path().join("agent-skills");
        fs::create_dir_all(&skill_root).expect("skill root");
        let destination = skill_root.join("code-review");
        let staging = skill_root.join(".skillhive-stage-tx1");
        let backup = skill_root.join(".skillhive-backup-tx1");
        make_skill(&destination, "old");
        make_skill(&staging, "new");
        fs::rename(&destination, &backup).expect("move old");

        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");
        let journal = DeploymentJournal {
            transaction_id: "tx1".to_owned(),
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "claude-code:default".to_owned(),
            blob_hash: "sha256:new".to_owned(),
            skill_root: skill_root.clone(),
            destination: destination.clone(),
            staging,
            backup,
            replaced_existing: true,
            phase: DeploymentPhase::OldMoved,
        };
        engine
            .persist_journal(&engine.journal_path("tx1"), &journal)
            .expect("journal");

        let report = engine.recover_incomplete().expect("recover");
        assert_eq!(report.recovered, 1);
        assert_eq!(report.catalog_commits.len(), 1);
        assert_eq!(report.catalog_commits[0].target_path, destination);
        assert!(engine.journal_path("tx1").exists());
        engine.acknowledge_catalog_commit("tx1").expect("ack");
    }

    #[test]
    fn intent_recovery_removes_partial_staging() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill_root = temp.path().join("agent-skills");
        fs::create_dir_all(&skill_root).expect("skill root");
        let staging = skill_root.join(".skillhive-stage-tx1");
        fs::create_dir_all(&staging).expect("staging");
        fs::write(staging.join("partial"), b"partial").expect("partial");
        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");
        let journal = DeploymentJournal {
            transaction_id: "tx1".to_owned(),
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "claude-code:default".to_owned(),
            blob_hash: "sha256:new".to_owned(),
            skill_root: skill_root.clone(),
            destination: skill_root.join("code-review"),
            staging: staging.clone(),
            backup: skill_root.join(".skillhive-backup-tx1"),
            replaced_existing: false,
            phase: DeploymentPhase::Intent,
        };
        engine
            .persist_journal(&engine.journal_path("tx1"), &journal)
            .expect("journal");

        let report = engine.recover_incomplete().expect("recover");
        assert_eq!(report.rolled_back, 1);
        assert!(!staging.exists());
        assert!(!engine.journal_path("tx1").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_inside_skill_tree_and_cleans_journal() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        make_skill(&source, "v1");
        symlink("/tmp", source.join("escape")).expect("symlink");
        let skill_root = temp.path().join("agent-skills");
        let journal_root = temp.path().join("journals");
        let engine = DeploymentEngine::open(&journal_root).expect("engine");

        let result = engine.deploy(request(source, skill_root));
        assert!(matches!(result, Err(DeploymentError::SymlinkNotAllowed(_))));
        assert!(fs::read_dir(journal_root)
            .expect("journals")
            .filter_map(Result::ok)
            .next()
            .is_none());
    }

    #[test]
    fn append_only_journal_uses_last_complete_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");
        let skill_root = temp.path().join("agent-skills");
        fs::create_dir_all(&skill_root).expect("skill root");
        let mut journal = DeploymentJournal {
            transaction_id: "tx1".to_owned(),
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "claude-code:default".to_owned(),
            blob_hash: "sha256:new".to_owned(),
            skill_root: skill_root.clone(),
            destination: skill_root.join("code-review"),
            staging: skill_root.join(".skillhive-stage-tx1"),
            backup: skill_root.join(".skillhive-backup-tx1"),
            replaced_existing: false,
            phase: DeploymentPhase::Intent,
        };
        let path = engine.journal_path("tx1");
        engine.persist_journal(&path, &journal).expect("intent");
        journal.phase = DeploymentPhase::Prepared;
        engine.persist_journal(&path, &journal).expect("prepared");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open")
            .write_all(b"{\"transaction_id\":")
            .expect("torn write");

        let recovered = read_latest_journal(&path).expect("latest");
        assert_eq!(recovered.phase, DeploymentPhase::Prepared);
    }
}
