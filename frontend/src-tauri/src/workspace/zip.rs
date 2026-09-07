//! Zip import/export at the client boundary (group-tree design doc §6).
//!
//! The zip archive is a user-facing packaging format only: import unpacks it
//! into a staging directory, reuses every `skill_snapshot` rule (SKILL.md
//! entrypoint, portable filenames, no symlinks, file count/size bounds) and
//! then enters the canonical content-addressed snapshot path. Export
//! materializes the verified snapshot from the blob store — never the
//! mutable workspace — and packs it. The server and the sync protocol are
//! untouched.
//!
//! Security posture: the WebView never supplies filesystem paths. Both
//! flows open native dialogs inside Rust; entry names are validated against
//! traversal/absolute/UNC forms and extraction is bounded before any bytes
//! are written (decompression-bomb defense), with the snapshot policy
//! re-validating everything afterwards.

use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use serde::Serialize;

use crate::{
    blob_store::BlobStore,
    skill_snapshot::{capture_workspace, materialize_snapshot, SkillSnapshotRef},
};

use super::{WorkspaceError, WorkspaceRef, WorkspaceStore};

/// Mirrors `SnapshotPolicy::default()`; the authoritative bounds are still
/// enforced by `capture_workspace` — these guard the staging extraction so
/// a hostile archive cannot exhaust disk before validation runs.
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ZipError {
    #[error("zip archive exceeds the {limit}-byte package limit")]
    ArchiveTooLarge { limit: u64 },
    #[error("zip archive contains too many entries (limit {limit})")]
    TooManyEntries { limit: usize },
    #[error("unsafe zip entry path: {0:?}")]
    UnsafeEntry(String),
    #[error("zip entry exceeds the per-file size limit: {path:?} (limit {limit})")]
    EntryTooLarge { path: String, limit: u64 },
    #[error("zip archive exceeds the total package size limit")]
    TotalTooLarge,
    #[error("zip entry size does not match its declared size: {path:?}")]
    EntrySizeMismatch { path: String },
    #[error("zip archive is not a readable skill package: {0}")]
    Malformed(String),
    #[error("zip filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] crate::skill_snapshot::SnapshotError),
    #[error("workspace error: {0}")]
    Workspace(#[from] WorkspaceError),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipImportOutcome {
    pub workspace: WorkspaceRef,
    pub manifest_hash: String,
    pub source_file_name: String,
    pub file_count: usize,
}

fn validate_entry_name(name: &str) -> Result<(), ZipError> {
    if name.is_empty() || name.contains('\\') {
        // Zip stores '/' separators; backslashes mean a non-portable
        // Windows-style writer and are rejected outright.
        return Err(ZipError::UnsafeEntry(name.to_owned()));
    }
    let path = Path::new(name);
    if path.is_absolute() {
        return Err(ZipError::UnsafeEntry(name.to_owned()));
    }
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.is_empty() || text == "." || text == ".." {
                    return Err(ZipError::UnsafeEntry(name.to_owned()));
                }
            }
            // Prefix (drive/UNC), RootDir, CurDir and ParentDir are all
            // rejected: entries must be plain relative names.
            _ => return Err(ZipError::UnsafeEntry(name.to_owned())),
        }
    }
    Ok(())
}

fn extract_archive(
    archive: &mut zip::ZipArchive<std::io::BufReader<std::fs::File>>,
    staging: &Path,
) -> Result<usize, ZipError> {
    if archive.len() > MAX_ENTRIES {
        return Err(ZipError::TooManyEntries { limit: MAX_ENTRIES });
    }
    let mut total_bytes: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| ZipError::Malformed(error.to_string()))?;
        let name = entry.name().to_owned();
        validate_entry_name(&name)?;
        if entry.is_dir() {
            fs::create_dir_all(staging.join(&name))?;
            continue;
        }
        // Symlink entries (unix file-type mode bits) are never materialized.
        if let Some(mode) = entry.unix_mode() {
            if mode & 0o170000 == 0o120000 {
                return Err(ZipError::UnsafeEntry(name));
            }
        }
        let declared = entry.size();
        if declared > MAX_ENTRY_BYTES {
            return Err(ZipError::EntryTooLarge {
                path: name,
                limit: MAX_ENTRY_BYTES,
            });
        }
        total_bytes += declared;
        if total_bytes > MAX_TOTAL_BYTES {
            return Err(ZipError::TotalTooLarge);
        }
        let target = staging.join(&name);
        if !target.starts_with(staging) {
            return Err(ZipError::UnsafeEntry(name));
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::new();
        (&mut entry).take(MAX_ENTRY_BYTES).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != declared {
            return Err(ZipError::EntrySizeMismatch { path: name });
        }
        fs::write(&target, &bytes)?;
    }
    Ok(archive.len())
}

/// Imports a user-selected zip archive into a fresh managed workspace.
///
/// The caller receives the captured manifest hash; queuing the create
/// mutation (outbox) is the caller's next step via the standard commit
/// path, exactly like any other local skill.
pub fn import_skill_zip(
    blobs: &BlobStore,
    workspaces: &WorkspaceStore,
    zip_path: &Path,
    skill_id: &str,
) -> Result<ZipImportOutcome, ZipError> {
    let metadata = fs::symlink_metadata(zip_path)?;
    if metadata.file_type().is_symlink() {
        return Err(ZipError::UnsafeEntry(
            zip_path.to_string_lossy().into_owned(),
        ));
    }
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err(ZipError::ArchiveTooLarge {
            limit: MAX_TOTAL_BYTES,
        });
    }

    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file))
        .map_err(|error| ZipError::Malformed(error.to_string()))?;

    let staging = std::env::temp_dir().join(format!(
        "skillhive-import-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&staging)?;
    let outcome = (|| {
        let file_count = extract_archive(&mut archive, &staging)?;
        let snapshot = capture_workspace(blobs, &staging, Default::default())?;
        let workspace = workspaces.import_snapshot(blobs, skill_id, &snapshot.manifest_hash)?;
        Ok(ZipImportOutcome {
            workspace,
            manifest_hash: snapshot.manifest_hash,
            source_file_name: zip_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            file_count,
        })
    })();
    let _ = fs::remove_dir_all(&staging);
    outcome
}

/// Packs a verified snapshot (materialized from the content-addressed blob
/// store, never the mutable workspace) into a zip archive at `destination`.
pub fn export_skill_zip(
    blobs: &BlobStore,
    snapshot_hash: &str,
    destination: &Path,
) -> Result<(), ZipError> {
    // Materialize first: blob reads are digest-verified and the snapshot
    // manifest is validated against the default policy before packing.
    // materialize_snapshot requires a non-existent absolute destination and
    // creates it itself.
    let staging = std::env::temp_dir().join(format!(
        "skillhive-export-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        let _snapshot: SkillSnapshotRef = materialize_snapshot(blobs, snapshot_hash, &staging)?;
        write_zip_tree(&staging, destination)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging);
    result
}

fn write_zip_tree(source: &Path, destination: &Path) -> Result<(), ZipError> {
    let file = fs::File::create(destination)?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    let mut paths: Vec<PathBuf> = Vec::new();
    collect_tree(source, source, &mut paths)?;
    paths.sort();
    for relative in paths {
        let absolute = source.join(&relative);
        let metadata = fs::symlink_metadata(&absolute)?;
        // Zip entry names always use '/' separators; Windows PathBuf's
        // string form uses '\' and would break portability.
        let name = relative.to_string_lossy().replace('\\', "/");
        if metadata.is_dir() {
            writer
                .add_directory(name, options)
                .map_err(|error| ZipError::Malformed(error.to_string()))?;
        } else {
            let bytes = fs::read(&absolute)?;
            writer
                .start_file(name, options)
                .map_err(|error| ZipError::Malformed(error.to_string()))?;
            use std::io::Write as _;
            writer.write_all(&bytes)?;
        }
    }
    writer
        .finish()
        .map_err(|error| ZipError::Malformed(error.to_string()))?;
    Ok(())
}

fn collect_tree(root: &Path, current: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = entry.metadata()?;
        if metadata.is_symlink() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unexpected symlink in verified snapshot",
            ));
        }
        let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        if metadata.is_dir() {
            out.push(relative.clone());
            collect_tree(root, &path, out)?;
        } else {
            out.push(relative);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn temp_root() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "skillhive-zip-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir).expect("temp root");
        dir
    }

    fn write_zip(path: &Path, entries: &[(&str, Vec<u8>)]) {
        let file = fs::File::create(path).expect("create zip file");
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for (name, bytes) in entries {
            writer
                .start_file((*name).to_owned(), options)
                .expect("start entry");
            use std::io::Write as _;
            writer.write_all(bytes).expect("write entry");
        }
        writer.finish().expect("finish zip");
    }

    fn store() -> (BlobStore, WorkspaceStore, PathBuf) {
        let root = temp_root();
        let blobs = BlobStore::open(root.join("blobs")).expect("blobs");
        let workspaces = WorkspaceStore::open(root.join("workspaces")).expect("workspaces");
        (blobs, workspaces, root)
    }

    #[test]
    fn entry_name_validation_rejects_traversal_and_absolute_paths() {
        assert!(validate_entry_name("SKILL.md").is_ok());
        assert!(validate_entry_name("scripts/run.sh").is_ok());
        assert!(validate_entry_name("../escape").is_err());
        assert!(validate_entry_name("/absolute").is_err());
        assert!(validate_entry_name("C:/evil").is_err());
        assert!(validate_entry_name("a\\b.txt").is_err());
        assert!(validate_entry_name("dir/../../out").is_err());
        assert!(validate_entry_name("").is_err());
        assert!(validate_entry_name("./leading").is_err());
    }

    #[test]
    fn import_rejects_zip_slip_entry() {
        let (blobs, workspaces, root) = store();
        let hostile = root.join("hostile.zip");
        write_zip(&hostile, &[("../escape.md", b"gone".to_vec())]);
        let error = import_skill_zip(&blobs, &workspaces, &hostile, "skill-1")
            .expect_err("zip-slip entry must be rejected");
        assert!(matches!(error, ZipError::UnsafeEntry(_)));
    }

    #[test]
    fn import_requires_skill_md_entrypoint() {
        let (blobs, workspaces, root) = store();
        let zip_path = root.join("no-entrypoint.zip");
        write_zip(&zip_path, &[("NOT_SKILL.md", b"hello".to_vec())]);
        let error = import_skill_zip(&blobs, &workspaces, &zip_path, "skill-2")
            .expect_err("missing SKILL.md must fail at capture");
        assert!(matches!(error, ZipError::Snapshot(_)));
    }

    #[test]
    fn import_rejects_entry_above_per_file_limit() {
        let (blobs, workspaces, root) = store();
        let zip_path = root.join("oversize.zip");
        let file = fs::File::create(&zip_path).expect("zip");
        let mut writer = zip::ZipWriter::new(file);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("SKILL.md", options).expect("start entry");
        use std::io::Write as _;
        writer
            .write_all(b"---\nname: big\ndescription: oversized entry\n---\n\nbody")
            .expect("write entry");
        writer.start_file("big.bin", options).expect("start big");
        // Deflated zeros compress to almost nothing, so producing an entry
        // whose declared size is one byte over the per-file limit is cheap.
        let chunk = vec![0u8; 1024 * 1024];
        for _ in 0..=(MAX_ENTRY_BYTES / chunk.len() as u64) {
            writer.write_all(&chunk).expect("write chunk");
        }
        writer.finish().expect("finish zip");

        let error = import_skill_zip(&blobs, &workspaces, &zip_path, "skill-big")
            .expect_err("entry above the per-file limit must be rejected");
        assert!(matches!(error, ZipError::EntryTooLarge { .. }));
    }

    #[test]
    fn import_and_export_round_trip() {
        let (blobs, workspaces, root) = store();
        let zip_path = root.join("skill.zip");
        write_zip(
            &zip_path,
            &[
                (
                    "SKILL.md",
                    b"---\nname: demo\ndescription: demo skill\n---\n\nbody".to_vec(),
                ),
                ("references/notes.md", b"# notes".to_vec()),
                ("scripts/run.sh", b"echo hi".to_vec()),
            ],
        );

        let outcome = import_skill_zip(&blobs, &workspaces, &zip_path, "skill-3").expect("import");
        assert_eq!(outcome.file_count, 3);
        assert_eq!(outcome.source_file_name, "skill.zip");
        assert!(
            workspaces
                .get("skill-3")
                .expect("workspace lookup")
                .is_some(),
            "managed workspace must exist"
        );

        let exported = root.join("exported.zip");
        export_skill_zip(&blobs, &outcome.manifest_hash, &exported).expect("export");
        let bytes = fs::read(&exported).expect("export bytes");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open");
        let names: Vec<String> = (0..archive.len())
            .map(|index| archive.by_index(index).expect("entry").name().to_owned())
            .collect();
        assert!(names.contains(&"SKILL.md".to_owned()));
        assert!(names.contains(&"references/notes.md".to_owned()));
        assert!(names.contains(&"scripts/run.sh".to_owned()));
    }
}
