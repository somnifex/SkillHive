use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const RESERVED_PREFIX: &str = ".skillhive-remove-";

#[derive(Debug, Clone)]
pub struct UninstallEngine {
    journal_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct UninstallRequest {
    pub skill_id: String,
    pub agent_profile_id: String,
    pub skill_root: PathBuf,
    pub target_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallResult {
    pub transaction_id: Option<String>,
    pub skill_id: String,
    pub agent_profile_id: String,
    pub target_path: PathBuf,
    pub quarantined_path: Option<PathBuf>,
    pub target_existed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UninstallPhase {
    Intent,
    Moved,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UninstallJournal {
    transaction_id: String,
    skill_id: String,
    agent_profile_id: String,
    skill_root: PathBuf,
    target_path: PathBuf,
    quarantined_path: PathBuf,
    phase: UninstallPhase,
}

impl UninstallJournal {
    fn result(&self) -> UninstallResult {
        UninstallResult {
            transaction_id: Some(self.transaction_id.clone()),
            skill_id: self.skill_id.clone(),
            agent_profile_id: self.agent_profile_id.clone(),
            target_path: self.target_path.clone(),
            quarantined_path: Some(self.quarantined_path.clone()),
            target_existed: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UninstallRecoveryReport {
    pub pending: Vec<UninstallResult>,
    pub cleaned_intents: usize,
    pub failed: Vec<String>,
}

impl UninstallEngine {
    pub fn open(journal_root: impl AsRef<Path>) -> Result<Self, UninstallError> {
        let journal_root = journal_root.as_ref().to_path_buf();
        fs::create_dir_all(&journal_root)?;
        Ok(Self { journal_root })
    }

    /// Begins an uninstall filesystem transaction.
    ///
    /// Existing content is renamed to a private quarantine path instead of
    /// being deleted. The caller must remove the matching SQLite deployment row
    /// and then call `finalize`. If the process dies before the catalog commit,
    /// startup recovery can atomically restore the quarantine to its target.
    pub fn begin(&self, request: UninstallRequest) -> Result<UninstallResult, UninstallError> {
        validate_request(&request)?;

        if !request.target_path.exists() {
            return Ok(UninstallResult {
                transaction_id: None,
                skill_id: request.skill_id,
                agent_profile_id: request.agent_profile_id,
                target_path: request.target_path,
                quarantined_path: None,
                target_existed: false,
            });
        }

        let transaction_id = Uuid::new_v4().to_string();
        let quarantined_path = request
            .skill_root
            .join(format!("{RESERVED_PREFIX}{transaction_id}"));
        let journal_path = self.journal_path(&transaction_id);
        let mut journal = UninstallJournal {
            transaction_id: transaction_id.clone(),
            skill_id: request.skill_id.clone(),
            agent_profile_id: request.agent_profile_id.clone(),
            skill_root: request.skill_root.clone(),
            target_path: request.target_path.clone(),
            quarantined_path: quarantined_path.clone(),
            phase: UninstallPhase::Intent,
        };
        self.persist_journal(&journal_path, &journal)?;

        fs::rename(&request.target_path, &quarantined_path)?;
        sync_directory(&request.skill_root)?;
        journal.phase = UninstallPhase::Moved;
        self.persist_journal(&journal_path, &journal)?;
        Ok(journal.result())
    }

    /// Permanently removes quarantined content after the SQLite deployment row
    /// has been committed absent.
    pub fn finalize(&self, transaction_id: &str) -> Result<(), UninstallError> {
        validate_transaction_id(transaction_id)?;
        let journal_path = self.journal_path(transaction_id);
        let journal = read_latest_journal(&journal_path)?;
        validate_journal_paths(&journal)?;
        if journal.transaction_id != transaction_id {
            return Err(UninstallError::InvalidJournal(
                "transaction id does not match journal filename".to_owned(),
            ));
        }
        if journal.target_path.exists() && journal.quarantined_path.exists() {
            return Err(UninstallError::AmbiguousFilesystemState(transaction_id.to_owned()));
        }
        if journal.target_path.exists() {
            return Err(UninstallError::CatalogCommitNotReflected(transaction_id.to_owned()));
        }
        if journal.quarantined_path.exists() {
            remove_any(&journal.quarantined_path)?;
            sync_directory(&journal.skill_root)?;
        }
        self.remove_journal(&journal_path)
    }

    /// Restores quarantined content when startup sees that the deployment row
    /// still exists, proving the SQLite half of uninstall never committed.
    pub fn rollback(&self, transaction_id: &str) -> Result<(), UninstallError> {
        validate_transaction_id(transaction_id)?;
        let journal_path = self.journal_path(transaction_id);
        let journal = read_latest_journal(&journal_path)?;
        validate_journal_paths(&journal)?;

        let target_exists = journal.target_path.exists();
        let quarantine_exists = journal.quarantined_path.exists();
        match (target_exists, quarantine_exists) {
            (false, true) => {
                fs::rename(&journal.quarantined_path, &journal.target_path)?;
                sync_directory(&journal.skill_root)?;
            }
            (true, false) => {}
            (true, true) => {
                return Err(UninstallError::AmbiguousFilesystemState(
                    transaction_id.to_owned(),
                ))
            }
            (false, false) => {
                return Err(UninstallError::UnrecoverableTransaction(
                    transaction_id.to_owned(),
                ))
            }
        }
        self.remove_journal(&journal_path)
    }

    /// Reads interrupted uninstall transactions without deciding whether to
    /// commit or roll back. SQLite catalog presence is the deciding authority
    /// and is intentionally evaluated by the Desktop Core, not this filesystem
    /// module.
    pub fn recover_pending(&self) -> Result<UninstallRecoveryReport, UninstallError> {
        fs::create_dir_all(&self.journal_root)?;
        let mut report = UninstallRecoveryReport {
            pending: Vec::new(),
            cleaned_intents: 0,
            failed: Vec::new(),
        };
        let mut journals = fs::read_dir(&self.journal_root)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("uninstall"))
            .collect::<Vec<_>>();
        journals.sort();

        for path in journals {
            let outcome = (|| -> Result<Option<UninstallResult>, UninstallError> {
                let journal = read_latest_journal(&path)?;
                validate_journal_paths(&journal)?;
                let target_exists = journal.target_path.exists();
                let quarantine_exists = journal.quarantined_path.exists();

                match (target_exists, quarantine_exists) {
                    (true, false) if journal.phase == UninstallPhase::Intent => {
                        self.remove_journal(&path)?;
                        report.cleaned_intents += 1;
                        Ok(None)
                    }
                    (false, true) => Ok(Some(journal.result())),
                    (false, false) if journal.phase == UninstallPhase::Moved => {
                        // Final deletion may have completed after SQLite commit
                        // but before journal removal. Catalog reconciliation can
                        // safely decide whether the journal should be cleaned.
                        Ok(Some(journal.result()))
                    }
                    _ => Err(UninstallError::AmbiguousFilesystemState(
                        journal.transaction_id,
                    )),
                }
            })();

            match outcome {
                Ok(Some(item)) => report.pending.push(item),
                Ok(None) => {}
                Err(error) => report
                    .failed
                    .push(format!("{}: {error}", path.display())),
            }
        }
        Ok(report)
    }

    pub fn acknowledge_missing_quarantine(
        &self,
        transaction_id: &str,
    ) -> Result<(), UninstallError> {
        validate_transaction_id(transaction_id)?;
        let path = self.journal_path(transaction_id);
        let journal = read_latest_journal(&path)?;
        validate_journal_paths(&journal)?;
        if journal.target_path.exists() || journal.quarantined_path.exists() {
            return Err(UninstallError::AmbiguousFilesystemState(
                transaction_id.to_owned(),
            ));
        }
        self.remove_journal(&path)
    }

    fn journal_path(&self, transaction_id: &str) -> PathBuf {
        self.journal_root
            .join(format!("{transaction_id}.uninstall"))
    }

    fn persist_journal(
        &self,
        path: &Path,
        journal: &UninstallJournal,
    ) -> Result<(), UninstallError> {
        let existed = path.exists();
        let bytes = serde_json::to_vec(journal)?;
        let mut file = OpenOptions::new()
            .write(true)
            .append(true)
            .create(true)
            .open(path)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        if !existed {
            sync_directory(&self.journal_root)?;
        }
        Ok(())
    }

    fn remove_journal(&self, path: &Path) -> Result<(), UninstallError> {
        if path.exists() {
            fs::remove_file(path)?;
            sync_directory(&self.journal_root)?;
        }
        Ok(())
    }
}

fn validate_request(request: &UninstallRequest) -> Result<(), UninstallError> {
    if request.skill_id.trim().is_empty() || request.agent_profile_id.trim().is_empty() {
        return Err(UninstallError::InvalidRequest(
            "skill_id and agent_profile_id must not be empty".to_owned(),
        ));
    }
    if !request.skill_root.is_absolute() || !request.target_path.is_absolute() {
        return Err(UninstallError::InvalidRequest(
            "skill_root and target_path must be absolute".to_owned(),
        ));
    }
    if request.target_path.parent() != Some(request.skill_root.as_path()) {
        return Err(UninstallError::InvalidRequest(
            "target_path must be a direct child of skill_root".to_owned(),
        ));
    }
    if !request.skill_root.is_dir() {
        return Err(UninstallError::InvalidRequest(
            "skill_root must exist and be a directory".to_owned(),
        ));
    }
    Ok(())
}

fn validate_journal_paths(journal: &UninstallJournal) -> Result<(), UninstallError> {
    if !journal.skill_root.is_absolute()
        || journal.target_path.parent() != Some(journal.skill_root.as_path())
        || journal.quarantined_path.parent() != Some(journal.skill_root.as_path())
    {
        return Err(UninstallError::InvalidJournal(
            "journal contains path outside skill root".to_owned(),
        ));
    }
    Ok(())
}

fn validate_transaction_id(value: &str) -> Result<(), UninstallError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(UninstallError::InvalidTransactionId(value.to_owned()));
    }
    Ok(())
}

fn read_latest_journal(path: &Path) -> Result<UninstallJournal, UninstallError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut latest = None;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<UninstallJournal>(&line) {
            Ok(record) => latest = Some(record),
            Err(_) if latest.is_some() => break,
            Err(error) => return Err(UninstallError::JournalSerialization(error)),
        }
    }
    latest.ok_or_else(|| {
        UninstallError::InvalidJournal(format!(
            "{} contains no complete journal record",
            path.display()
        ))
    })
}

fn remove_any(path: &Path) -> Result<(), UninstallError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        return Err(UninstallError::InvalidJournal(format!(
            "unsupported quarantined file type at {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), UninstallError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), UninstallError> {
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum UninstallError {
    #[error("uninstall filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("uninstall journal serialization error: {0}")]
    JournalSerialization(#[from] serde_json::Error),
    #[error("invalid uninstall request: {0}")]
    InvalidRequest(String),
    #[error("invalid uninstall transaction id: {0}")]
    InvalidTransactionId(String),
    #[error("invalid uninstall journal: {0}")]
    InvalidJournal(String),
    #[error("uninstall transaction {0} has ambiguous filesystem state")]
    AmbiguousFilesystemState(String),
    #[error("uninstall transaction {0} cannot be rolled back")]
    UnrecoverableTransaction(String),
    #[error("uninstall transaction {0} still has an active target")]
    CatalogCommitNotReflected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_then_rollback_restores_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("skills");
        let target = root.join("demo");
        fs::create_dir_all(&target).expect("target");
        fs::write(target.join("SKILL.md"), b"demo").expect("skill");
        let engine = UninstallEngine::open(temp.path().join("journals")).expect("engine");
        let result = engine
            .begin(UninstallRequest {
                skill_id: "skill-1".to_owned(),
                agent_profile_id: "custom:test".to_owned(),
                skill_root: root,
                target_path: target.clone(),
            })
            .expect("begin");
        assert!(!target.exists());
        engine
            .rollback(result.transaction_id.as_deref().expect("tx"))
            .expect("rollback");
        assert!(target.join("SKILL.md").exists());
    }

    #[test]
    fn begin_then_finalize_removes_quarantine() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("skills");
        let target = root.join("demo");
        fs::create_dir_all(&target).expect("target");
        fs::write(target.join("SKILL.md"), b"demo").expect("skill");
        let engine = UninstallEngine::open(temp.path().join("journals")).expect("engine");
        let result = engine
            .begin(UninstallRequest {
                skill_id: "skill-1".to_owned(),
                agent_profile_id: "custom:test".to_owned(),
                skill_root: root,
                target_path: target.clone(),
            })
            .expect("begin");
        engine
            .finalize(result.transaction_id.as_deref().expect("tx"))
            .expect("finalize");
        assert!(!target.exists());
        assert!(!result.quarantined_path.expect("quarantine").exists());
    }
}
