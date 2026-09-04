use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{
    blob_store::{BlobStore, BlobStoreError},
    skill_snapshot::{read_manifest, SkillSnapshotRef, SnapshotError},
};

pub fn verify_materialized_snapshot(
    blobs: &BlobStore,
    manifest_hash: &str,
    root: &Path,
) -> Result<SkillSnapshotRef, SnapshotVerificationError> {
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| SnapshotVerificationError::InvalidRoot(root.to_path_buf()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SnapshotVerificationError::InvalidRoot(root.to_path_buf()));
    }

    let manifest = read_manifest(blobs, manifest_hash)?;
    let mut actual = Vec::new();
    collect_files(root, root, &mut actual)?;
    actual.sort_by(|left, right| left.0.cmp(&right.0));

    if actual.len() != manifest.files.len() {
        return Err(SnapshotVerificationError::FileSetMismatch {
            expected: manifest.files.len(),
            actual: actual.len(),
        });
    }

    let mut total_bytes = 0_u64;
    for (expected, (actual_path, actual_file)) in manifest.files.iter().zip(actual.into_iter()) {
        if expected.path != actual_path {
            return Err(SnapshotVerificationError::PathMismatch {
                expected: expected.path.clone(),
                actual: actual_path,
            });
        }

        let metadata = fs::symlink_metadata(&actual_file)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(SnapshotVerificationError::UnsupportedFileType(actual_file));
        }
        if metadata.len() != expected.size_bytes {
            return Err(SnapshotVerificationError::SizeMismatch {
                path: expected.path.clone(),
                expected: expected.size_bytes,
                actual: metadata.len(),
            });
        }

        let actual_hash = hash_file_stable(&actual_file, expected.size_bytes)?;
        if actual_hash != expected.blob_hash {
            return Err(SnapshotVerificationError::HashMismatch {
                path: expected.path.clone(),
                expected: expected.blob_hash.clone(),
                actual: actual_hash,
            });
        }

        total_bytes = total_bytes
            .checked_add(expected.size_bytes)
            .ok_or(SnapshotVerificationError::SizeOverflow)?;
    }

    Ok(SkillSnapshotRef {
        manifest_hash: manifest_hash.to_owned(),
        file_count: manifest.files.len(),
        total_bytes,
    })
}

fn collect_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<(String, PathBuf)>,
) -> Result<(), SnapshotVerificationError> {
    let mut entries = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            return Err(SnapshotVerificationError::SymlinkNotAllowed(path));
        }
        if file_type.is_dir() {
            collect_files(root, &path, output)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(SnapshotVerificationError::UnsupportedFileType(path));
        }

        let relative = path
            .strip_prefix(root)
            .map_err(|_| SnapshotVerificationError::PathEscapedRoot(path.clone()))?;
        output.push((relative_to_portable_path(relative)?, path));
    }
    Ok(())
}

fn relative_to_portable_path(path: &Path) -> Result<String, SnapshotVerificationError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| SnapshotVerificationError::NonUtf8Path(path.to_path_buf()))?;
                if value.is_empty() {
                    return Err(SnapshotVerificationError::InvalidPath(
                        path.display().to_string(),
                    ));
                }
                parts.push(value.to_owned());
            }
            _ => {
                return Err(SnapshotVerificationError::InvalidPath(
                    path.display().to_string(),
                ))
            }
        }
    }
    if parts.is_empty() {
        return Err(SnapshotVerificationError::InvalidPath(String::new()));
    }
    Ok(parts.join("/"))
}

fn hash_file_stable(
    path: &Path,
    expected_size: u64,
) -> Result<String, SnapshotVerificationError> {
    let before = fs::symlink_metadata(path)?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() != expected_size {
        return Err(SnapshotVerificationError::SourceChangedDuringVerification(
            path.to_path_buf(),
        ));
    }

    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or(SnapshotVerificationError::SizeOverflow)?;
        hasher.update(&buffer[..read]);
    }
    if total != expected_size {
        return Err(SnapshotVerificationError::SourceChangedDuringVerification(
            path.to_path_buf(),
        ));
    }

    let after = fs::symlink_metadata(path)?;
    if after.file_type().is_symlink() || !after.is_file() || after.len() != expected_size {
        return Err(SnapshotVerificationError::SourceChangedDuringVerification(
            path.to_path_buf(),
        ));
    }

    let digest = hasher.finalize();
    let mut output = String::from("sha256:");
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(output)
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotVerificationError {
    #[error("snapshot verification filesystem error: {0}")]
    Io(#[from] io::Error),
    #[error("blob store error: {0}")]
    Blob(#[from] BlobStoreError),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("invalid materialized snapshot root: {0:?}")]
    InvalidRoot(PathBuf),
    #[error("symbolic links are not allowed in deployed snapshots: {0:?}")]
    SymlinkNotAllowed(PathBuf),
    #[error("unsupported file type in deployed snapshot: {0:?}")]
    UnsupportedFileType(PathBuf),
    #[error("deployed path escaped snapshot root: {0:?}")]
    PathEscapedRoot(PathBuf),
    #[error("path is not valid UTF-8: {0:?}")]
    NonUtf8Path(PathBuf),
    #[error("deployed file changed during verification: {0:?}")]
    SourceChangedDuringVerification(PathBuf),
    #[error("invalid deployed relative path: {0}")]
    InvalidPath(String),
    #[error("deployed file set mismatch: expected {expected} files, got {actual}")]
    FileSetMismatch { expected: usize, actual: usize },
    #[error("deployed path mismatch: expected {expected}, got {actual}")]
    PathMismatch { expected: String, actual: String },
    #[error("deployed file size mismatch for {path}: expected {expected}, got {actual}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("deployed file hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("snapshot size overflow during verification")]
    SizeOverflow,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skill_snapshot::{capture_workspace, materialize_snapshot, SnapshotPolicy};

    fn make_workspace(root: &Path) {
        fs::create_dir_all(root.join("scripts")).expect("mkdir");
        fs::write(root.join("SKILL.md"), b"# Skill\n").expect("skill");
        fs::write(root.join("scripts").join("run.py"), b"print('ok')\n").expect("script");
    }

    #[test]
    fn exact_snapshot_verifies() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let materialized = temp.path().join("materialized");
        make_workspace(&workspace);
        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");
        let snapshot = capture_workspace(&blobs, &workspace, SnapshotPolicy::default())
            .expect("snapshot");
        materialize_snapshot(&blobs, &snapshot.manifest_hash, &materialized)
            .expect("materialize");

        assert_eq!(
            verify_materialized_snapshot(&blobs, &snapshot.manifest_hash, &materialized)
                .expect("verify"),
            snapshot
        );
    }

    #[test]
    fn extra_file_is_detected() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("workspace");
        let materialized = temp.path().join("materialized");
        make_workspace(&workspace);
        let blobs = BlobStore::open(temp.path().join("blobs")).expect("blobs");
        let snapshot = capture_workspace(&blobs, &workspace, SnapshotPolicy::default())
            .expect("snapshot");
        materialize_snapshot(&blobs, &snapshot.manifest_hash, &materialized)
            .expect("materialize");
        fs::write(materialized.join("unexpected.txt"), b"changed").expect("write");

        assert!(matches!(
            verify_materialized_snapshot(&blobs, &snapshot.manifest_hash, &materialized),
            Err(SnapshotVerificationError::FileSetMismatch { .. })
        ));
    }
}
