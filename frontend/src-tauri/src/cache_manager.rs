use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    blob_store::{BlobStore, BlobStoreError},
    local_store::{CacheSkillRecord, LocalStore, LocalStoreError, SkillSyncState},
    skill_snapshot::{read_manifest, SnapshotError},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEnforcementReport {
    pub budget_bytes: u64,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub evicted_skills: Vec<String>,
    pub deleted_blobs: usize,
    pub skipped_skills: Vec<String>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone)]
struct BlobInventoryEntry {
    hash: String,
    path: PathBuf,
    size_bytes: u64,
}

pub fn enforce_cache_budget(
    store: &LocalStore,
    blobs: &BlobStore,
) -> Result<CacheEnforcementReport, CacheManagerError> {
    let policy = store.cache_policy()?;
    let skills = store.list_cache_skills()?;
    let deployments = store.list_deployments()?;
    let mutation_payloads = store.list_unacked_mutation_payload_hashes()?;
    let inventory = scan_blob_inventory(blobs.root())?;
    let before_bytes = inventory
        .iter()
        .try_fold(0_u64, |total, entry| total.checked_add(entry.size_bytes))
        .ok_or(CacheManagerError::SizeOverflow)?;

    let mut report = CacheEnforcementReport {
        budget_bytes: policy.max_bytes,
        before_bytes,
        after_bytes: before_bytes,
        evicted_skills: Vec::new(),
        deleted_blobs: 0,
        skipped_skills: Vec::new(),
        diagnostics: Vec::new(),
    };

    if before_bytes <= policy.max_bytes {
        return Ok(report);
    }

    let deployed_skill_ids: HashSet<String> = deployments
        .iter()
        .map(|deployment| deployment.skill_id.clone())
        .collect();

    // Count snapshot roots, not just unique hashes. Two skills can legally share
    // an identical immutable snapshot; an outbox mutation can also reference an
    // older snapshot no longer current on any skill. Every reference contributes
    // one unit so evicting one skill can never delete another owner's bytes.
    let mut snapshot_reference_counts: HashMap<String, usize> = HashMap::new();
    for skill in &skills {
        if skill.sync_state != SkillSyncState::RemoteOnly {
            *snapshot_reference_counts
                .entry(skill.snapshot_hash.clone())
                .or_default() += 1;
        }
    }
    for deployment in &deployments {
        *snapshot_reference_counts
            .entry(deployment.deployed_blob_hash.clone())
            .or_default() += 1;
    }
    for payload_hash in mutation_payloads {
        *snapshot_reference_counts.entry(payload_hash).or_default() += 1;
    }

    let mut closures: HashMap<String, HashSet<String>> = HashMap::new();
    let mut unresolved_snapshots = HashSet::new();
    for snapshot_hash in snapshot_reference_counts.keys() {
        match snapshot_closure(blobs, snapshot_hash) {
            Ok(closure) => {
                closures.insert(snapshot_hash.clone(), closure);
            }
            Err(error) => {
                unresolved_snapshots.insert(snapshot_hash.clone());
                report.diagnostics.push(format!(
                    "snapshot {snapshot_hash} could not be resolved for cache accounting: {error}"
                ));
            }
        }
    }

    let mut reference_counts: HashMap<String, usize> = HashMap::new();
    for (snapshot_hash, snapshot_refs) in &snapshot_reference_counts {
        let Some(closure) = closures.get(snapshot_hash) else {
            continue;
        };
        for hash in closure {
            *reference_counts.entry(hash.clone()).or_default() += *snapshot_refs;
        }
    }

    let mut candidates = skills;
    candidates.sort_by(|left, right| {
        left.last_accessed_at
            .cmp(&right.last_accessed_at)
            .then_with(|| left.skill_id.cmp(&right.skill_id))
    });

    let inventory_by_hash: HashMap<String, BlobInventoryEntry> = inventory
        .into_iter()
        .map(|entry| (entry.hash.clone(), entry))
        .collect();

    for skill in candidates {
        if report.after_bytes <= policy.max_bytes {
            break;
        }

        if !is_evictable(&skill, &deployed_skill_ids) {
            report.skipped_skills.push(skill.skill_id);
            continue;
        }
        if unresolved_snapshots.contains(&skill.snapshot_hash) {
            report.skipped_skills.push(skill.skill_id);
            continue;
        }
        let Some(closure) = closures.get(&skill.snapshot_hash).cloned() else {
            report.skipped_skills.push(skill.skill_id);
            continue;
        };

        if skill.workspace_path.exists() {
            report.skipped_skills.push(skill.skill_id);
            continue;
        }

        if !store.claim_skill_for_eviction(&skill.skill_id, &skill.snapshot_hash)? {
            report.skipped_skills.push(skill.skill_id);
            continue;
        }

        report.evicted_skills.push(skill.skill_id.clone());
        for hash in closure {
            let Some(count) = reference_counts.get_mut(&hash) else {
                continue;
            };
            *count = count.saturating_sub(1);
            if *count != 0 {
                continue;
            }

            let Some(entry) = inventory_by_hash.get(&hash) else {
                continue;
            };
            match remove_blob_file(blobs, &entry.hash, &entry.path) {
                Ok(true) => {
                    report.after_bytes = report.after_bytes.saturating_sub(entry.size_bytes);
                    report.deleted_blobs += 1;
                }
                Ok(false) => {}
                Err(error) => report.diagnostics.push(format!(
                    "skill {} was marked remote-only but cached blob {} could not be deleted: {error}",
                    skill.skill_id, entry.hash
                )),
            }
        }
    }

    // Orphan sweeping is intentionally disabled if even one referenced manifest
    // is unreadable. In that state we cannot prove which file blobs are live.
    if report.after_bytes > policy.max_bytes && unresolved_snapshots.is_empty() {
        let referenced: HashSet<&str> = reference_counts
            .iter()
            .filter_map(|(hash, count)| (*count > 0).then_some(hash.as_str()))
            .collect();
        for entry in inventory_by_hash.values() {
            if report.after_bytes <= policy.max_bytes {
                break;
            }
            if referenced.contains(entry.hash.as_str()) {
                continue;
            }
            match remove_blob_file(blobs, &entry.hash, &entry.path) {
                Ok(true) => {
                    report.after_bytes = report.after_bytes.saturating_sub(entry.size_bytes);
                    report.deleted_blobs += 1;
                }
                Ok(false) => {}
                Err(error) => report
                    .diagnostics
                    .push(format!("orphan blob {} could not be deleted: {error}", entry.hash)),
            }
        }
    }

    if report.after_bytes > policy.max_bytes {
        report.diagnostics.push(format!(
            "cache remains above budget: {} bytes used, {} byte budget; remaining content is protected or unresolved",
            report.after_bytes, policy.max_bytes
        ));
    }

    Ok(report)
}

fn is_evictable(skill: &CacheSkillRecord, deployed_skill_ids: &HashSet<String>) -> bool {
    skill.sync_state == SkillSyncState::Synced
        && !skill.pinned
        && skill.remote_id.is_some()
        && !deployed_skill_ids.contains(&skill.skill_id)
        && !skill.workspace_path.exists()
}

fn snapshot_closure(
    blobs: &BlobStore,
    manifest_hash: &str,
) -> Result<HashSet<String>, CacheManagerError> {
    let manifest = read_manifest(blobs, manifest_hash)?;
    let mut closure = HashSet::with_capacity(manifest.files.len() + 1);
    closure.insert(manifest_hash.to_owned());
    for file in manifest.files {
        closure.insert(file.blob_hash);
    }
    Ok(closure)
}

fn scan_blob_inventory(root: &Path) -> Result<Vec<BlobInventoryEntry>, CacheManagerError> {
    let sha_root = root.join("sha256");
    if !sha_root.exists() {
        return Ok(Vec::new());
    }

    let mut inventory = Vec::new();
    for prefix_entry in fs::read_dir(&sha_root)? {
        let prefix_entry = prefix_entry?;
        let prefix_path = prefix_entry.path();
        if !prefix_entry.file_type()?.is_dir() {
            continue;
        }
        let prefix = prefix_entry.file_name().to_string_lossy().to_string();
        if prefix.len() != 2 || !is_lower_hex(&prefix) {
            continue;
        }

        for blob_entry in fs::read_dir(prefix_path)? {
            let blob_entry = blob_entry?;
            if !blob_entry.file_type()?.is_file() {
                continue;
            }
            let digest = blob_entry.file_name().to_string_lossy().to_string();
            if digest.len() != 64 || !is_lower_hex(&digest) || !digest.starts_with(&prefix) {
                continue;
            }
            let metadata = blob_entry.metadata()?;
            inventory.push(BlobInventoryEntry {
                hash: format!("sha256:{digest}"),
                path: blob_entry.path(),
                size_bytes: metadata.len(),
            });
        }
    }
    inventory.sort_by(|left, right| left.hash.cmp(&right.hash));
    Ok(inventory)
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn remove_blob_file(
    blobs: &BlobStore,
    hash: &str,
    expected_path: &Path,
) -> Result<bool, CacheManagerError> {
    let canonical = blobs.path_for_hash(hash)?;
    if canonical != expected_path {
        return Err(CacheManagerError::InventoryPathMismatch {
            hash: hash.to_owned(),
            expected: canonical,
            actual: expected_path.to_path_buf(),
        });
    }

    match fs::remove_file(expected_path) {
        Ok(()) => {
            sync_parent(expected_path.parent())?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(CacheManagerError::Io(error)),
    }
}

#[cfg(unix)]
fn sync_parent(parent: Option<&Path>) -> Result<(), CacheManagerError> {
    use std::fs::File;
    if let Some(parent) = parent {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_parent: Option<&Path>) -> Result<(), CacheManagerError> {
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum CacheManagerError {
    #[error("cache filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("local store error: {0}")]
    LocalStore(#[from] LocalStoreError),
    #[error("blob store error: {0}")]
    BlobStore(#[from] BlobStoreError),
    #[error("snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("cache size overflow")]
    SizeOverflow,
    #[error("blob inventory path mismatch for {hash}: expected {expected:?}, got {actual:?}")]
    InventoryPathMismatch {
        hash: String,
        expected: PathBuf,
        actual: PathBuf,
    },
}
