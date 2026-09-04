pub mod import;

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    blob_store::BlobStore,
    skill_snapshot::{materialize_snapshot, SnapshotError},
};

const MAX_INITIAL_SKILL_MD_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct WorkspaceStore {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRef {
    pub skill_id: String,
    pub path: PathBuf,
}

impl WorkspaceStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let root = root.as_ref().to_path_buf();
        ensure_directory(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Maps an arbitrary logical skill id to a fixed, traversal-free local path.
    pub fn path_for_skill(&self, skill_id: &str) -> Result<PathBuf, WorkspaceError> {
        validate_skill_id(skill_id)?;
        let mut hasher = Sha256::new();
        hasher.update(skill_id.as_bytes());
        let digest = hasher.finalize();
        let mut name = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write as _;
            write!(&mut name, "{byte:02x}").expect("writing to String cannot fail");
        }
        Ok(self.root.join(name))
    }

    pub fn create(
        &self,
        skill_id: &str,
        initial_skill_md: &str,
    ) -> Result<WorkspaceRef, WorkspaceError> {
        validate_skill_id(skill_id)?;
        if initial_skill_md.len() > MAX_INITIAL_SKILL_MD_BYTES {
            return Err(WorkspaceError::InitialSkillTooLarge {
                size: initial_skill_md.len(),
                limit: MAX_INITIAL_SKILL_MD_BYTES,
            });
        }

        let path = self.path_for_skill(skill_id)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(WorkspaceError::InvalidWorkspace(path));
                }
                let entrypoint = path.join("SKILL.md");
                if fs::symlink_metadata(&entrypoint).is_ok() {
                    return Err(WorkspaceError::AlreadyExists(path));
                }
                if fs::read_dir(&path)?.next().is_some() {
                    return Err(WorkspaceError::IncompleteWorkspace(path));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path)?;
                sync_directory(&self.root)?;
            }
            Err(error) => return Err(WorkspaceError::Io(error)),
        }

        write_new_file_durable(&path.join("SKILL.md"), initial_skill_md.as_bytes())?;
        sync_directory(&path)?;
        Ok(WorkspaceRef {
            skill_id: skill_id.to_owned(),
            path,
        })
    }

    /// Materializes an already-validated immutable snapshot into a new managed
    /// workspace. The caller owns source authorization/discovery; this method
    /// only guarantees that the destination is inside SkillHive's workspace
    /// root and is created atomically by snapshot materialization semantics.
    pub fn import_snapshot(
        &self,
        blobs: &BlobStore,
        skill_id: &str,
        snapshot_hash: &str,
    ) -> Result<WorkspaceRef, WorkspaceError> {
        validate_skill_id(skill_id)?;
        let path = self.path_for_skill(skill_id)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(WorkspaceError::AlreadyExists(path)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(WorkspaceError::Io(error)),
        }

        materialize_snapshot(blobs, snapshot_hash, &path)?;
        sync_directory(&self.root)?;
        Ok(WorkspaceRef {
            skill_id: skill_id.to_owned(),
            path,
        })
    }

    pub fn get(&self, skill_id: &str) -> Result<Option<WorkspaceRef>, WorkspaceError> {
        let path = self.path_for_skill(skill_id)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(WorkspaceError::InvalidWorkspace(path));
                }
                Ok(Some(WorkspaceRef {
                    skill_id: skill_id.to_owned(),
                    path,
                }))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(WorkspaceError::Io(error)),
        }
    }

    /// Removes only a SkillHive-managed workspace. The caller must prove the
    /// workspace is clean and synchronized before invoking this method.
    pub fn remove(&self, skill_id: &str) -> Result<bool, WorkspaceError> {
        let path = self.path_for_skill(skill_id)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(WorkspaceError::InvalidWorkspace(path));
                }
                remove_tree_without_following_symlinks(&path)?;
                sync_directory(&self.root)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(WorkspaceError::Io(error)),
        }
    }
}

fn validate_skill_id(skill_id: &str) -> Result<(), WorkspaceError> {
    if skill_id.trim().is_empty() || skill_id.len() > 512 {
        return Err(WorkspaceError::InvalidSkillId);
    }
    Ok(())
}

fn ensure_directory(path: &Path) -> Result<(), WorkspaceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(WorkspaceError::InvalidWorkspaceRoot(path.to_path_buf()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(WorkspaceError::InvalidWorkspaceRoot(path.to_path_buf()));
            }
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
        }
        Err(error) => return Err(WorkspaceError::Io(error)),
    }
    Ok(())
}

fn write_new_file_durable(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let parent = path
        .parent()
        .ok_or_else(|| WorkspaceError::InvalidWorkspace(path.to_path_buf()))?;
    let temporary = parent.join(format!(".skillhive-create-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<(), WorkspaceError> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        fs::remove_file(&temporary).ok();
    }
    result
}

fn remove_tree_without_following_symlinks(path: &Path) -> Result<(), WorkspaceError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(WorkspaceError::InvalidWorkspace(path.to_path_buf()));
    }

    for entry in fs::read_dir(path)? {
        let entry = entry?;
        remove_tree_without_following_symlinks(&entry.path())?;
    }
    fs::remove_dir(path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), WorkspaceError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), WorkspaceError> {
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("workspace filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot materialization error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("invalid skill id")]
    InvalidSkillId,
    #[error("invalid workspace root: {0:?}")]
    InvalidWorkspaceRoot(PathBuf),
    #[error("invalid managed workspace: {0:?}")]
    InvalidWorkspace(PathBuf),
    #[error("managed workspace already exists: {0:?}")]
    AlreadyExists(PathBuf),
    #[error("managed workspace is incomplete and contains unexpected files: {0:?}")]
    IncompleteWorkspace(PathBuf),
    #[error("initial SKILL.md has {size} bytes, exceeding limit {limit}")]
    InitialSkillTooLarge { size: usize, limit: usize },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_snapshot::{capture_workspace, SnapshotPolicy};

    #[test]
    fn logical_ids_cannot_escape_workspace_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspaces = WorkspaceStore::open(temp.path().join("workspaces")).expect("store");
        let path = workspaces.path_for_skill("../../etc/passwd").expect("path");
        assert_eq!(path.parent(), Some(workspaces.root()));
    }

    #[test]
    fn create_and_remove_managed_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspaces = WorkspaceStore::open(temp.path().join("workspaces")).expect("store");
        let created = workspaces.create("skill-1", "# Demo\n").expect("create");
        assert_eq!(fs::read(created.path.join("SKILL.md")).expect("read"), b"# Demo\n");
        assert!(workspaces.remove("skill-1").expect("remove"));
        assert!(workspaces.get("skill-1").expect("get").is_none());
    }

    #[test]
    fn immutable_snapshot_can_be_imported_into_managed_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        fs::create_dir_all(&source).expect("source");
        fs::write(source.join("SKILL.md"), b"# Imported\n").expect("skill");
        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");
        let snapshot = capture_workspace(&blobs, &source, SnapshotPolicy::default()).expect("snapshot");
        let workspaces = WorkspaceStore::open(temp.path().join("workspaces")).expect("store");

        let imported = workspaces
            .import_snapshot(&blobs, "skill-imported", &snapshot.manifest_hash)
            .expect("import");
        assert_eq!(
            fs::read(imported.path.join("SKILL.md")).expect("read"),
            b"# Imported\n"
        );
    }
}
