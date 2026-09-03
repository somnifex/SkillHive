use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::blob_store::{BlobRef, BlobStore, BlobStoreError};

const SNAPSHOT_FORMAT_VERSION: u32 = 1;
const SKILL_ENTRYPOINT: &str = "SKILL.md";

#[derive(Debug, Clone, Copy)]
pub struct SnapshotPolicy {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for SnapshotPolicy {
    fn default() -> Self {
        Self {
            max_files: 10_000,
            max_file_bytes: 64 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSnapshotManifest {
    pub format_version: u32,
    pub files: Vec<SkillSnapshotFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSnapshotFile {
    /// Portable relative path with `/` separators.
    pub path: String,
    pub blob_hash: String,
    pub size_bytes: u64,
    /// Unix permission bits when captured on Unix; absent on other platforms.
    pub unix_mode: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSnapshotRef {
    pub manifest_hash: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

pub fn capture_workspace(
    blobs: &BlobStore,
    workspace: &Path,
    policy: SnapshotPolicy,
) -> Result<SkillSnapshotRef, SnapshotError> {
    validate_workspace_root(workspace)?;

    let mut source_files = Vec::new();
    collect_files(workspace, workspace, &mut source_files, policy)?;
    source_files.sort_by(|left, right| left.0.cmp(&right.0));

    if !source_files
        .iter()
        .any(|(relative, _)| relative == SKILL_ENTRYPOINT)
    {
        return Err(SnapshotError::MissingEntrypoint(
            workspace.join(SKILL_ENTRYPOINT),
        ));
    }

    let mut total_bytes = 0_u64;
    let mut files = Vec::with_capacity(source_files.len());
    for (relative, source) in source_files {
        let metadata = fs::metadata(&source)?;
        let size = metadata.len();
        if size > policy.max_file_bytes {
            return Err(SnapshotError::FileTooLarge {
                path: source,
                size,
                limit: policy.max_file_bytes,
            });
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or(SnapshotError::SizeOverflow)?;
        if total_bytes > policy.max_total_bytes {
            return Err(SnapshotError::SnapshotTooLarge {
                size: total_bytes,
                limit: policy.max_total_bytes,
            });
        }

        let bytes = fs::read(&source)?;
        let blob = blobs.put_bytes(&bytes)?;
        files.push(SkillSnapshotFile {
            path: relative,
            blob_hash: blob.hash,
            size_bytes: size,
            unix_mode: unix_mode(&metadata),
        });
    }

    let manifest = SkillSnapshotManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        files,
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    let manifest_blob = blobs.put_bytes(&manifest_bytes)?;

    Ok(SkillSnapshotRef {
        manifest_hash: manifest_blob.hash,
        file_count: manifest.files.len(),
        total_bytes,
    })
}

pub fn read_manifest(
    blobs: &BlobStore,
    manifest_hash: &str,
) -> Result<SkillSnapshotManifest, SnapshotError> {
    let bytes = blobs.read_bytes(manifest_hash)?;
    let manifest: SkillSnapshotManifest = serde_json::from_slice(&bytes)?;
    if manifest.format_version != SNAPSHOT_FORMAT_VERSION {
        return Err(SnapshotError::UnsupportedFormat(manifest.format_version));
    }
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Materializes an immutable snapshot to a new directory. The destination must
/// not exist; deployment code owns the final atomic activation step.
pub fn materialize_snapshot(
    blobs: &BlobStore,
    manifest_hash: &str,
    destination: &Path,
) -> Result<SkillSnapshotRef, SnapshotError> {
    if destination.exists() {
        return Err(SnapshotError::DestinationExists(destination.to_path_buf()));
    }
    if !destination.is_absolute() {
        return Err(SnapshotError::InvalidDestination(destination.to_path_buf()));
    }

    let manifest = read_manifest(blobs, manifest_hash)?;
    fs::create_dir_all(destination)?;
    let result = (|| -> Result<SkillSnapshotRef, SnapshotError> {
        let mut total_bytes = 0_u64;
        for file in &manifest.files {
            let relative = portable_path_to_relative(&file.path)?;
            let target = destination.join(relative);
            let parent = target
                .parent()
                .ok_or_else(|| SnapshotError::InvalidManifestPath(file.path.clone()))?;
            fs::create_dir_all(parent)?;

            let bytes = blobs.read_bytes(&file.blob_hash)?;
            if bytes.len() as u64 != file.size_bytes {
                return Err(SnapshotError::BlobSizeMismatch {
                    path: file.path.clone(),
                    expected: file.size_bytes,
                    actual: bytes.len() as u64,
                });
            }
            let mut target_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)?;
            target_file.write_all(&bytes)?;
            apply_unix_mode(&target_file, file.unix_mode)?;
            target_file.sync_all()?;
            total_bytes = total_bytes
                .checked_add(file.size_bytes)
                .ok_or(SnapshotError::SizeOverflow)?;
        }
        sync_tree_directories(destination)?;
        Ok(SkillSnapshotRef {
            manifest_hash: manifest_hash.to_owned(),
            file_count: manifest.files.len(),
            total_bytes,
        })
    })();

    if result.is_err() {
        fs::remove_dir_all(destination).ok();
    }
    result
}

fn collect_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<(String, PathBuf)>,
    policy: SnapshotPolicy,
) -> Result<(), SnapshotError> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(SnapshotError::SymlinkNotAllowed(path));
        }
        if file_type.is_dir() {
            collect_files(root, &path, output, policy)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(SnapshotError::UnsupportedFileType(path));
        }

        if output.len() >= policy.max_files {
            return Err(SnapshotError::TooManyFiles {
                limit: policy.max_files,
            });
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| SnapshotError::PathEscapedRoot(path.clone()))?;
        output.push((relative_to_portable_path(relative)?, path));
    }
    Ok(())
}

fn validate_workspace_root(path: &Path) -> Result<(), SnapshotError> {
    if !path.is_absolute() {
        return Err(SnapshotError::InvalidWorkspace(path.to_path_buf()));
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| SnapshotError::InvalidWorkspace(path.to_path_buf()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SnapshotError::InvalidWorkspace(path.to_path_buf()));
    }
    let entrypoint = path.join(SKILL_ENTRYPOINT);
    let metadata = fs::symlink_metadata(&entrypoint)
        .map_err(|_| SnapshotError::MissingEntrypoint(entrypoint.clone()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SnapshotError::MissingEntrypoint(entrypoint));
    }
    Ok(())
}

fn validate_manifest(manifest: &SkillSnapshotManifest) -> Result<(), SnapshotError> {
    let mut previous: Option<&str> = None;
    let mut has_entrypoint = false;
    for file in &manifest.files {
        portable_path_to_relative(&file.path)?;
        if previous.is_some_and(|value| value >= file.path.as_str()) {
            return Err(SnapshotError::ManifestNotStrictlySorted);
        }
        previous = Some(&file.path);
        if file.path == SKILL_ENTRYPOINT {
            has_entrypoint = true;
        }
        if !is_canonical_sha256(&file.blob_hash) {
            return Err(SnapshotError::InvalidBlobHash(file.blob_hash.clone()));
        }
    }
    if !has_entrypoint {
        return Err(SnapshotError::MissingEntrypoint(PathBuf::from(SKILL_ENTRYPOINT)));
    }
    Ok(())
}

fn relative_to_portable_path(path: &Path) -> Result<String, SnapshotError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| SnapshotError::NonUtf8Path(path.to_path_buf()))?;
                if value.is_empty() {
                    return Err(SnapshotError::InvalidManifestPath(path.display().to_string()));
                }
                parts.push(value.to_owned());
            }
            _ => return Err(SnapshotError::InvalidManifestPath(path.display().to_string())),
        }
    }
    if parts.is_empty() {
        return Err(SnapshotError::InvalidManifestPath(String::new()));
    }
    Ok(parts.join("/"))
}

fn portable_path_to_relative(value: &str) -> Result<PathBuf, SnapshotError> {
    if value.is_empty() || value.starts_with('/') || value.contains('\\') {
        return Err(SnapshotError::InvalidManifestPath(value.to_owned()));
    }
    let mut result = PathBuf::new();
    for part in value.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(SnapshotError::InvalidManifestPath(value.to_owned()));
        }
        result.push(part);
    }
    if result.is_absolute() {
        return Err(SnapshotError::InvalidManifestPath(value.to_owned()));
    }
    Ok(result)
}

fn is_canonical_sha256(value: &str) -> bool {
    let Some(digest) = value.strip_prefix("sha256:") else {
        return false;
    };
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(unix)]
fn unix_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn apply_unix_mode(file: &File, mode: Option<u32>) -> Result<(), SnapshotError> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(mode) = mode {
        file.set_permissions(fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn apply_unix_mode(_file: &File, _mode: Option<u32>) -> Result<(), SnapshotError> {
    Ok(())
}

fn sync_tree_directories(root: &Path) -> Result<(), SnapshotError> {
    #[cfg(unix)]
    {
        fn walk(path: &Path) -> io::Result<()> {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    walk(&entry.path())?;
                }
            }
            File::open(path)?.sync_all()?;
            Ok(())
        }
        walk(root)?;
    }
    #[cfg(not(unix))]
    {
        let _ = root;
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshot filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("blob store error: {0}")]
    Blob(#[from] BlobStoreError),
    #[error("snapshot manifest serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("invalid workspace: {0}")]
    InvalidWorkspace(PathBuf),
    #[error("missing or invalid SKILL.md: {0}")]
    MissingEntrypoint(PathBuf),
    #[error("symbolic links are not allowed in managed skill snapshots: {0}")]
    SymlinkNotAllowed(PathBuf),
    #[error("unsupported file type in skill snapshot: {0}")]
    UnsupportedFileType(PathBuf),
    #[error("skill path escaped workspace root: {0}")]
    PathEscapedRoot(PathBuf),
    #[error("path is not valid UTF-8: {0}")]
    NonUtf8Path(PathBuf),
    #[error("snapshot has more than {limit} files")]
    TooManyFiles { limit: usize },
    #[error("file {path} has {size} bytes, exceeding limit {limit}")]
    FileTooLarge { path: PathBuf, size: u64, limit: u64 },
    #[error("snapshot size {size} exceeds limit {limit}")]
    SnapshotTooLarge { size: u64, limit: u64 },
    #[error("snapshot size overflow")]
    SizeOverflow,
    #[error("unsupported snapshot format version {0}")]
    UnsupportedFormat(u32),
    #[error("invalid manifest path: {0}")]
    InvalidManifestPath(String),
    #[error("snapshot manifest paths are not strictly sorted and unique")]
    ManifestNotStrictlySorted,
    #[error("invalid blob hash in snapshot manifest: {0}")]
    InvalidBlobHash(String),
    #[error("snapshot materialization destination already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("snapshot materialization destination must be absolute: {0}")]
    InvalidDestination(PathBuf),
    #[error("blob size mismatch for {path}: expected {expected}, got {actual}")]
    BlobSizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_workspace(root: &Path) {
        fs::create_dir_all(root.join("scripts")).expect("mkdir");
        fs::write(root.join("SKILL.md"), b"# Skill\n").expect("skill");
        fs::write(root.join("scripts").join("run.py"), b"print('ok')\n").expect("script");
    }

    #[test]
    fn snapshot_round_trip_is_deterministic() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        make_workspace(&workspace);
        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");

        let first = capture_workspace(&blobs, &workspace, SnapshotPolicy::default()).expect("capture");
        let second = capture_workspace(&blobs, &workspace, SnapshotPolicy::default()).expect("capture");
        assert_eq!(first, second);

        let output = temp.path().join("materialized");
        let materialized = materialize_snapshot(&blobs, &first.manifest_hash, &output).expect("materialize");
        assert_eq!(materialized, first);
        assert_eq!(fs::read(output.join("SKILL.md")).expect("read"), b"# Skill\n");
        assert_eq!(
            fs::read(output.join("scripts").join("run.py")).expect("read"),
            b"print('ok')\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_rejects_symlink() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        make_workspace(&workspace);
        symlink("/tmp", workspace.join("escape")).expect("symlink");
        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");
        assert!(matches!(
            capture_workspace(&blobs, &workspace, SnapshotPolicy::default()),
            Err(SnapshotError::SymlinkNotAllowed(_))
        ));
    }

    #[test]
    fn manifest_rejects_parent_traversal() {
        let manifest = SkillSnapshotManifest {
            format_version: SNAPSHOT_FORMAT_VERSION,
            files: vec![SkillSnapshotFile {
                path: "../escape".to_owned(),
                blob_hash: format!("sha256:{}", "a".repeat(64)),
                size_bytes: 1,
                unix_mode: None,
            }],
        };
        assert!(matches!(
            validate_manifest(&manifest),
            Err(SnapshotError::InvalidManifestPath(_))
        ));
    }
}
