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
/// only local durability: stage -> validate -> journal -> swap -> cleanup.
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
    phase: DeploymentPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryReport {
    pub recovered: usize,
    pub rolled_back: usize,
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
    /// The source tree is never modified. Symlinks are rejected in the first
    /// enterprise implementation so an imported skill cannot escape its source
    /// tree or cause writes outside the selected agent skill root.
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

        let staging_result = (|| -> Result<(), DeploymentError> {
            copy_tree_durable(&request.source_dir, &staging)?;
            validate_source_skill(&staging)?;
            sync_directory(&request.skill_root)?;
            Ok(())
        })();
        if let Err(error) = staging_result {
            if staging.exists() {
                remove_any(&staging).ok();
                sync_directory(&request.skill_root).ok();
            }
            return Err(error);
        }

        let mut journal = DeploymentJournal {
            transaction_id: transaction_id.clone(),
            skill_id: request.skill_id.clone(),
            agent_profile_id: request.agent_profile_id.clone(),
            blob_hash: request.blob_hash.clone(),
            skill_root: request.skill_root.clone(),
            destination: destination.clone(),
            staging: staging.clone(),
            backup: backup.clone(),
            phase: DeploymentPhase::Prepared,
        };
        self.persist_journal(&journal_path, &journal)?;

        let replaced_existing = destination.exists();
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
            self.remove_journal(&journal_path)?;
            Ok(())
        })();

        if let Err(error) = swap_result {
            // Do not perform an ad-hoc destructive rollback here. The durable
            // append-only journal is the source for deterministic recovery after
            // any error or process interruption.
            return Err(error);
        }

        Ok(DeploymentResult {
            transaction_id,
            skill_id: request.skill_id,
            agent_profile_id: request.agent_profile_id,
            target_path: destination,
            blob_hash: request.blob_hash,
            replaced_existing,
        })
    }

    /// Reconciles interrupted filesystem transactions before new deployments.
    ///
    /// Recovery prefers a fully materialized staged version when the old target
    /// has already been moved to backup; otherwise it restores the previous
    /// target. A journal is removed only after the filesystem reaches a stable
    /// state and the containing directory is synchronized where supported.
    pub fn recover_incomplete(&self) -> Result<RecoveryReport, DeploymentError> {
        fs::create_dir_all(&self.journal_root)?;
        let mut report = RecoveryReport {
            recovered: 0,
            rolled_back: 0,
            failed: Vec::new(),
        };

        let mut journals = fs::read_dir(&self.journal_root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("journal"))
            .collect::<Vec<_>>();
        journals.sort();

        for journal_path in journals {
            let result = self.recover_one(&journal_path);
            match result {
                Ok(RecoveryDisposition::Committed) => report.recovered += 1,
                Ok(RecoveryDisposition::RolledBack) => report.rolled_back += 1,
                Err(error) => report
                    .failed
                    .push(format!("{}: {error}", journal_path.display())),
            }
        }

        Ok(report)
    }

    fn recover_one(&self, journal_path: &Path) -> Result<RecoveryDisposition, DeploymentError> {
        let journal = read_latest_journal(journal_path)?;
        validate_journal_paths(&journal)?;

        let destination_exists = journal.destination.exists();
        let staging_exists = journal.staging.exists();
        let backup_exists = journal.backup.exists();

        if destination_exists {
            if journal.phase == DeploymentPhase::Prepared && !backup_exists {
                // No old target was moved. The visible destination is therefore
                // still the previous version and the staged candidate was never
                // activated.
                if staging_exists {
                    remove_any(&journal.staging)?;
                    sync_directory(&journal.skill_root)?;
                }
                self.remove_journal(journal_path)?;
                return Ok(RecoveryDisposition::RolledBack);
            }

            if journal.phase == DeploymentPhase::OldMoved && staging_exists && backup_exists {
                return Err(DeploymentError::UnrecoverableTransaction {
                    transaction_id: journal.transaction_id,
                    reason: "active, staging, and backup paths all exist after old target was moved"
                        .to_owned(),
                });
            }

            // Activated may have completed before its phase record reached disk.
            // Destination present + staging absent is safe to treat as committed.
            if staging_exists {
                remove_any(&journal.staging)?;
            }
            if backup_exists {
                remove_any(&journal.backup)?;
            }
            sync_directory(&journal.skill_root)?;
            self.remove_journal(journal_path)?;
            return Ok(RecoveryDisposition::Committed);
        }

        if staging_exists && backup_exists {
            // Old target was moved but new target was not durably activated.
            // The staging tree was validated before the first journal record, so
            // roll forward to the new complete version.
            fs::rename(&journal.staging, &journal.destination)?;
            sync_directory(&journal.skill_root)?;
            remove_any(&journal.backup)?;
            sync_directory(&journal.skill_root)?;
            self.remove_journal(journal_path)?;
            return Ok(RecoveryDisposition::Committed);
        }

        if !staging_exists && backup_exists {
            // The new tree is unavailable; restore the last known-good target.
            fs::rename(&journal.backup, &journal.destination)?;
            sync_directory(&journal.skill_root)?;
            self.remove_journal(journal_path)?;
            return Ok(RecoveryDisposition::RolledBack);
        }

        if staging_exists && !backup_exists && journal.phase == DeploymentPhase::Prepared {
            // No active target was moved. Discard the unactivated staging tree.
            remove_any(&journal.staging)?;
            sync_directory(&journal.skill_root)?;
            self.remove_journal(journal_path)?;
            return Ok(RecoveryDisposition::RolledBack);
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

    /// Journal updates are append-only. Each complete phase record is followed
    /// by a newline and fsynced. A crash can at worst leave one torn trailing
    /// record; the previous complete phase remains available for recovery.
    fn persist_journal(
        &self,
        journal_path: &Path,
        journal: &DeploymentJournal,
    ) -> Result<(), DeploymentError> {
        let existed = journal_path.exists();
        let bytes = serde_json::to_vec(journal)?;
        let mut file = OpenOptions::new()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecoveryDisposition {
    Committed,
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
            Err(error) if latest.is_some() => {
                // An invalid trailing record is treated as an interrupted append.
                // Stop immediately; valid records after corruption are not
                // accepted because they would indicate external modification.
                let _ = error;
                break;
            }
            Err(error) => return Err(DeploymentError::JournalSerialization(error)),
        }
    }

    latest.ok_or_else(|| DeploymentError::InvalidJournal(format!(
        "{} contains no complete journal record",
        path.display()
    )))
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
    // File contents are flushed before activation. Windows directory handle
    // flushing is not portable through std; the recovery journal compensates
    // for interrupted multi-rename transactions.
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

    #[test]
    fn deploys_and_replaces_as_one_directory_version() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source_v1 = temp.path().join("source-v1");
        let source_v2 = temp.path().join("source-v2");
        let skill_root = temp.path().join("agent-skills");
        make_skill(&source_v1, "v1");
        make_skill(&source_v2, "v2");

        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");
        let first = engine
            .deploy(DeploymentRequest {
                skill_id: "skill-1".to_owned(),
                agent_profile_id: "claude:default".to_owned(),
                source_dir: source_v1,
                skill_root: skill_root.clone(),
                directory_name: "code-review".to_owned(),
                blob_hash: "sha256:v1".to_owned(),
            })
            .expect("first deploy");
        assert!(!first.replaced_existing);

        let second = engine
            .deploy(DeploymentRequest {
                skill_id: "skill-1".to_owned(),
                agent_profile_id: "claude:default".to_owned(),
                source_dir: source_v2,
                skill_root: skill_root.clone(),
                directory_name: "code-review".to_owned(),
                blob_hash: "sha256:v2".to_owned(),
            })
            .expect("replace");
        assert!(second.replaced_existing);
        assert_eq!(
            fs::read_to_string(skill_root.join("code-review").join("scripts/run.sh"))
                .expect("read"),
            "v2"
        );
    }

    #[test]
    fn rejects_path_traversal_directory_name() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        make_skill(&source, "v1");
        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");

        let result = engine.deploy(DeploymentRequest {
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "custom:default".to_owned(),
            source_dir: source,
            skill_root: temp.path().join("agent-skills"),
            directory_name: "../escape".to_owned(),
            blob_hash: "sha256:v1".to_owned(),
        });
        assert!(matches!(result, Err(DeploymentError::InvalidDirectoryName(_))));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_inside_skill_tree_and_cleans_staging() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        make_skill(&source, "v1");
        symlink("/tmp", source.join("escape")).expect("symlink");
        let skill_root = temp.path().join("agent-skills");
        let engine = DeploymentEngine::open(temp.path().join("journals")).expect("engine");

        let result = engine.deploy(DeploymentRequest {
            skill_id: "skill-1".to_owned(),
            agent_profile_id: "claude:default".to_owned(),
            source_dir: source,
            skill_root: skill_root.clone(),
            directory_name: "safe-name".to_owned(),
            blob_hash: "sha256:v1".to_owned(),
        });
        assert!(matches!(result, Err(DeploymentError::SymlinkNotAllowed(_))));
        let leftovers = fs::read_dir(skill_root)
            .expect("skill root")
            .filter_map(Result::ok)
            .collect::<Vec<_>>();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn recovery_rolls_forward_after_old_target_was_moved() {
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
            agent_profile_id: "claude:default".to_owned(),
            blob_hash: "sha256:new".to_owned(),
            skill_root: skill_root.clone(),
            destination: destination.clone(),
            staging: staging.clone(),
            backup: backup.clone(),
            phase: DeploymentPhase::OldMoved,
        };
        engine
            .persist_journal(&engine.journal_path("tx1"), &journal)
            .expect("journal");

        let report = engine.recover_incomplete().expect("recover");
        assert_eq!(report.recovered, 1);
        assert!(destination.exists());
        assert!(!backup.exists());
        assert!(!staging.exists());
        assert_eq!(
            fs::read_to_string(destination.join(SKILL_ENTRYPOINT)).expect("read"),
            "# new\n"
        );
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
            agent_profile_id: "claude:default".to_owned(),
            blob_hash: "sha256:new".to_owned(),
            skill_root: skill_root.clone(),
            destination: skill_root.join("code-review"),
            staging: skill_root.join(".skillhive-stage-tx1"),
            backup: skill_root.join(".skillhive-backup-tx1"),
            phase: DeploymentPhase::Prepared,
        };
        let path = engine.journal_path("tx1");
        engine.persist_journal(&path, &journal).expect("prepared");
        journal.phase = DeploymentPhase::OldMoved;
        engine.persist_journal(&path, &journal).expect("old moved");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open")
            .write_all(b"{\"transaction_id\":")
            .expect("torn write");

        let recovered = read_latest_journal(&path).expect("latest");
        assert_eq!(recovered.phase, DeploymentPhase::OldMoved);
    }
}
