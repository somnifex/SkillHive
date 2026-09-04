use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::blob_store::{BlobStore, BlobStoreError};

const SNAPSHOT_FORMAT_VERSION: u32 = 1;
const SKILL_ENTRYPOINT: &str = "SKILL.md";
const MAX_PORTABLE_SEGMENT_BYTES: usize = 255;

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
    pub path: String,
    pub blob_hash: String,
    pub size_bytes: u64,
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
        let bytes = read_regular_file_stable(&source)?;
        let size = u64::try_from(bytes.len()).map_err(|_| SnapshotError::SizeOverflow)?;
        enforce_file_size(&source, size, policy)?;
        total_bytes = add_and_enforce_total(total_bytes, size, policy)?;

        let blob = blobs.put_bytes(&bytes)?;
        files.push(SkillSnapshotFile {
            path: relative,
            blob_hash: blob.hash,
            size_bytes: size,
        });
    }

    let manifest = SkillSnapshotManifest {
        format_version: SNAPSHOT_FORMAT_VERSION,
        files,
    };
    validate_manifest(&manifest, policy)?;
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
    read_manifest_with_policy(blobs, manifest_hash, SnapshotPolicy::default())
}

pub fn read_manifest_with_policy(
    blobs: &BlobStore,
    manifest_hash: &str,
    policy: SnapshotPolicy,
) -> Result<SkillSnapshotManifest, SnapshotError> {
    let bytes = blobs.read_bytes(manifest_hash)?;
    let manifest: SkillSnapshotManifest = serde_json::from_slice(&bytes)?;
    if manifest.format_version != SNAPSHOT_FORMAT_VERSION {
        return Err(SnapshotError::UnsupportedFormat(manifest.format_version));
    }
    validate_manifest(&manifest, policy)?;
    Ok(manifest)
}

pub fn materialize_snapshot(
    blobs: &BlobStore,
    manifest_hash: &str,
    destination: &Path,
) -> Result<SkillSnapshotRef, SnapshotError> {
    materialize_snapshot_with_policy(blobs, manifest_hash, destination, SnapshotPolicy::default())
}

pub fn materialize_snapshot_with_policy(
    blobs: &BlobStore,
    manifest_hash: &str,
    destination: &Path,
    policy: SnapshotPolicy,
) -> Result<SkillSnapshotRef, SnapshotError> {
    if destination.exists() {
        return Err(SnapshotError::DestinationExists(destination.to_path_buf()));
    }
    if !destination.is_absolute() {
        return Err(SnapshotError::InvalidDestination(destination.to_path_buf()));
    }

    let manifest = read_manifest_with_policy(blobs, manifest_hash, policy)?;
    fs::create_dir_all(destination)?;
    let result = (|| -> Result<SkillSnapshotRef, SnapshotError> {
        let mut total_bytes = 0_u64;
        for file in &manifest.files {
            let relative = portable_path_to_relative(&file.path)?;
            let target = destination.join(relative);
            let parent = target
                .parent()
                .ok_or_else(|| SnapshotError::InvalidManifestPath(file.path.clone()))?;
            create_materialization_parent(destination, parent)?;

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
            target_file.sync_all()?;
            total_bytes = add_and_enforce_total(total_bytes, file.size_bytes, policy)?;
        }
        sync_tree_directories(destination)?;
        Ok(SkillSnapshotRef {
            manifest_hash: manifest_hash.to_owned(),
            file_count: manifest.files.len(),
            total_bytes,
        })
    })();

    if result.is_err() {
        remove_tree_without_following_symlinks(destination).ok();
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

fn read_regular_file_stable(path: &Path) -> Result<Vec<u8>, SnapshotError> {
    let before = fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_file() {
        return Err(SnapshotError::UnsupportedFileType(path.to_path_buf()));
    }
    let bytes = fs::read(path)?;
    let after = fs::symlink_metadata(path)?;
    if after.file_type().is_symlink() || !after.is_file() {
        return Err(SnapshotError::SourceChangedDuringCapture(path.to_path_buf()));
    }
    let byte_len = u64::try_from(bytes.len()).map_err(|_| SnapshotError::SizeOverflow)?;
    if before.len() != byte_len || after.len() != byte_len {
        return Err(SnapshotError::SourceChangedDuringCapture(path.to_path_buf()));
    }
    Ok(bytes)
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

fn validate_manifest(
    manifest: &SkillSnapshotManifest,
    policy: SnapshotPolicy,
) -> Result<(), SnapshotError> {
    if manifest.files.len() > policy.max_files {
        return Err(SnapshotError::TooManyFiles {
            limit: policy.max_files,
        });
    }

    let mut previous: Option<&str> = None;
    let mut has_entrypoint = false;
    let mut total_bytes = 0_u64;
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
        enforce_file_size(Path::new(&file.path), file.size_bytes, policy)?;
        total_bytes = add_and_enforce_total(total_bytes, file.size_bytes, policy)?;
    }
    if !has_entrypoint {
        return Err(SnapshotError::MissingEntrypoint(PathBuf::from(SKILL_ENTRYPOINT)));
    }
    Ok(())
}

fn enforce_file_size(
    path: &Path,
    size: u64,
    policy: SnapshotPolicy,
) -> Result<(), SnapshotError> {
    if size > policy.max_file_bytes {
        return Err(SnapshotError::FileTooLarge {
            path: path.to_path_buf(),
            size,
            limit: policy.max_file_bytes,
        });
    }
    Ok(())
}

fn add_and_enforce_total(
    current: u64,
    additional: u64,
    policy: SnapshotPolicy,
) -> Result<u64, SnapshotError> {
    let total = current
        .checked_add(additional)
        .ok_or(SnapshotError::SizeOverflow)?;
    if total > policy.max_total_bytes {
        return Err(SnapshotError::SnapshotTooLarge {
            size: total,
            limit: policy.max_total_bytes,
        });
    }
    Ok(total)
}

fn relative_to_portable_path(path: &Path) -> Result<String, SnapshotError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| SnapshotError::NonUtf8Path(path.to_path_buf()))?;
                validate_portable_segment(value)?;
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
        validate_portable_segment(part)?;
        result.push(part);
    }
    if result.is_absolute() {
        return Err(SnapshotError::InvalidManifestPath(value.to_owned()));
    }
    Ok(result)
}

fn validate_portable_segment(value: &str) -> Result<(), SnapshotError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.as_bytes().len() > MAX_PORTABLE_SEGMENT_BYTES
        || value.ends_with(' ')
        || value.ends_with('.')
        || value.chars().any(|character| {
            character.is_control()
                || matches!(character, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*')
        })
    {
        return Err(SnapshotError::InvalidManifestPath(value.to_owned()));
    }

    let stem = value.split('.').next().unwrap_or(value).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"))
        || stem
            .strip_prefix("LPT")
            .is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"));
    if reserved {
        return Err(SnapshotError::InvalidManifestPath(value.to_owned()));
    }
    Ok(())
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

fn create_materialization_parent(root: &Path, parent: &Path) -> Result<(), SnapshotError> {
    let relative = parent
        .strip_prefix(root)
        .map_err(|_| SnapshotError::PathEscapedRoot(parent.to_path_buf()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err(SnapshotError::InvalidManifestPath(
                relative.display().to_string(),
            ));
        };
        current.push(segment);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(SnapshotError::UnsafeMaterializationPath(current));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir(&current)?,
            Err(error) => return Err(SnapshotError::Io(error)),
        }
    }
    Ok(())
}

fn remove_tree_without_following_symlinks(path: &Path) -> Result<(), SnapshotError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(SnapshotError::UnsupportedFileType(path.to_path_buf()));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        remove_tree_without_following_symlinks(&entry.path())?;
    }
    fs::remove_dir(path)?;
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
    #[error("invalid workspace: {0:?}")]
    InvalidWorkspace(PathBuf),
    #[error("missing or invalid SKILL.md: {0:?}")]
    MissingEntrypoint(PathBuf),
    #[error("symbolic links are not allowed in managed skill snapshots: {0:?}")]
    SymlinkNotAllowed(PathBuf),
    #[error("unsupported file type in skill snapshot: {0:?}")]
    UnsupportedFileType(PathBuf),
    #[error("skill path escaped workspace root: {0:?}")]
    PathEscapedRoot(PathBuf),
    #[error("path is not valid UTF-8: {0:?}")]
    NonUtf8Path(PathBuf),
    #[error("source file changed while snapshot was being captured: {0:?}")]
    SourceChangedDuringCapture(PathBuf),
    #[error("snapshot has more than {limit} files")]
    TooManyFiles { limit: usize },
    #[error("file {path:?} has {size} bytes, exceeding limit {limit}")]
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
    #[error("snapshot materialization destination already exists: {0:?}")]
    DestinationExists(PathBuf),
    #[error("snapshot materialization destination must be absolute: {0:?}")]
    InvalidDestination(PathBuf),
    #[error("unsafe materialization path: {0:?}")]
    UnsafeMaterializationPath(PathBuf),
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
            }],
        };
        assert!(matches!(
            validate_manifest(&manifest, SnapshotPolicy::default()),
            Err(SnapshotError::InvalidManifestPath(_))
        ));
    }

    #[test]
    fn manifest_rejects_nonportable_windows_names() {
        for path in ["CON", "scripts/a:b.py", "trailing."] {
            assert!(portable_path_to_relative(path).is_err(), "{path}");
        }
    }

    #[test]
    fn manifest_limits_are_enforced_before_materialization() {
        let manifest = SkillSnapshotManifest {
            format_version: SNAPSHOT_FORMAT_VERSION,
            files: vec![SkillSnapshotFile {
                path: "SKILL.md".to_owned(),
                blob_hash: format!("sha256:{}", "a".repeat(64)),
                size_bytes: 10,
            }],
        };
        let policy = SnapshotPolicy {
            max_files: 10,
            max_file_bytes: 5,
            max_total_bytes: 100,
        };
        assert!(matches!(
            validate_manifest(&manifest, policy),
            Err(SnapshotError::FileTooLarge { .. })
        ));
    }
}
