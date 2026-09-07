//! On-demand hydration of pulled skills (M2.5 gap closure).
//!
//! The pull feed stores metadata plus the package manifest blob only, so a
//! `remote_only` record is not deployable: its file closure and managed
//! workspace do not exist yet. [`hydrate_skill`] completes exactly that
//! state — it downloads missing closure blobs through the same verified
//! download path as pull, materializes the snapshot into a managed
//! workspace, and only then promotes the local record. Every step is
//! additive: a failure at any point leaves the record `remote_only` and at
//! worst leaks recoverable disk space, never local work.

use std::path::PathBuf;

use crate::blob_store::BlobStoreError;
use crate::local_store::{LocalStore, LocalStoreError, SkillSyncState};
use crate::skill_snapshot::{read_manifest, SnapshotError};
use crate::snapshot_verifier::{verify_materialized_snapshot, SnapshotVerificationError};
use crate::sync_client::{SyncClient, SyncClientError};
use crate::workspace::{WorkspaceError, WorkspaceStore};

#[derive(Debug, thiserror::Error)]
pub enum HydrationError {
    #[error(transparent)]
    Http(#[from] SyncClientError),
    #[error("local store failure: {0}")]
    LocalStore(#[from] LocalStoreError),
    #[error("blob store failure: {0}")]
    BlobStore(#[from] BlobStoreError),
    #[error("snapshot failure: {0}")]
    Snapshot(#[from] SnapshotError),
    #[error("workspace failure: {0}")]
    Workspace(#[from] WorkspaceError),
    #[error("snapshot verification failed: {0}")]
    Verification(#[from] SnapshotVerificationError),
    #[error("blob download failed for {hash}: {source}")]
    BlobDownload {
        hash: String,
        #[source]
        source: Box<SyncClientError>,
    },
    #[error("downloaded blob {expected} does not match its content hash (got {actual})")]
    DigestMismatch { expected: String, actual: String },
    #[error("skill {0} in state {1} cannot be hydrated: hydrating it would race unsynchronized local work")]
    InvalidState(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct HydrationOutcome {
    pub skill_id: String,
    pub manifest_hash: String,
    pub blobs_downloaded: usize,
    pub workspace_created: bool,
    pub workspace_path: PathBuf,
}

impl SyncClient {
    fn download_blob(
        &self,
        blobs: &crate::blob_store::BlobStore,
        hash: &str,
    ) -> Result<(), HydrationError> {
        let bytes = self
            .get_octet_stream(&format!("/api/v1/sync/blobs/{hash}"))
            .map_err(|source| HydrationError::BlobDownload {
                hash: hash.to_owned(),
                source: Box::new(source),
            })?;
        let actual = crate::blob_store::hash_bytes_for_verification(&bytes);
        if actual != hash {
            return Err(HydrationError::DigestMismatch {
                expected: hash.to_owned(),
                actual,
            });
        }
        blobs.put_bytes(&bytes)?;
        Ok(())
    }
}

/// Downloads every blob referenced by `manifest_hash` that the local blob
/// store is still missing. The manifest itself counts as part of the
/// closure: a record can legitimately lack even its manifest bytes when it
/// was pulled before the download step, or when cache eviction removed the
/// clean manifest.
fn ensure_closure_local(
    client: &SyncClient,
    blobs: &crate::blob_store::BlobStore,
    manifest_hash: &str,
) -> Result<usize, HydrationError> {
    let mut downloaded = 0_usize;
    if !blobs.verify(manifest_hash)? {
        client.download_blob(blobs, manifest_hash)?;
        downloaded += 1;
    }

    let manifest = read_manifest(blobs, manifest_hash)?;
    for file in &manifest.files {
        if blobs.verify(&file.blob_hash)? {
            continue;
        }
        client.download_blob(blobs, &file.blob_hash)?;
        downloaded += 1;
    }
    Ok(downloaded)
}

/// Ensures a skill's snapshot closure and managed workspace exist locally,
/// promoting `remote_only` records to `synced` afterwards.
///
/// - `remote_only`: pull-created metadata-only record; full hydration.
/// - `synced` without workspace (workspace was released for cache budget):
///   re-materializes from the local blob store; the durable record stays
///   `synced`.
/// - every other state (dirty/conflict/...): refused, because a fresh
///   materialization could race unsynchronized local work.
pub fn hydrate_skill(
    client: &SyncClient,
    store: &LocalStore,
    blobs: &crate::blob_store::BlobStore,
    workspaces: &WorkspaceStore,
    skill_id: &str,
) -> Result<HydrationOutcome, HydrationError> {
    let skill = store
        .get_skill(skill_id)?
        .ok_or_else(|| LocalStoreError::SkillNotFound(skill_id.to_owned()))?;
    let manifest_hash = skill.current_blob_hash.clone();

    if let Some(workspace) = workspaces.get(skill_id)? {
        // Workspace already exists: never touch it, just re-verify it.
        verify_materialized_snapshot(blobs, &manifest_hash, &workspace.path)?;
        if matches!(skill.sync_state, SkillSyncState::RemoteOnly) {
            store.mark_skill_hydrated(skill_id, &workspace.path)?;
        }
        return Ok(HydrationOutcome {
            skill_id: skill.id,
            manifest_hash,
            blobs_downloaded: 0,
            workspace_created: false,
            workspace_path: workspace.path,
        });
    }

    if !matches!(
        skill.sync_state,
        SkillSyncState::RemoteOnly | SkillSyncState::Synced
    ) {
        return Err(HydrationError::LocalStore(LocalStoreError::InvalidInput(
            format!(
                "skill {skill_id} in state {:?} without a workspace cannot be hydrated",
                skill.sync_state
            ),
        )));
    }

    let blobs_downloaded = ensure_closure_local(client, blobs, &manifest_hash)?;
    let workspace = workspaces.import_snapshot(blobs, skill_id, &manifest_hash)?;
    if matches!(skill.sync_state, SkillSyncState::RemoteOnly) {
        store.mark_skill_hydrated(skill_id, &workspace.path)?;
    }

    Ok(HydrationOutcome {
        skill_id: skill.id,
        manifest_hash,
        blobs_downloaded,
        workspace_created: true,
        workspace_path: workspace.path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_store::{ChangeItem, ChangesPage};
    use crate::skill_snapshot::capture_workspace;
    use serde_json::json;

    fn offline_client() -> SyncClient {
        // Never contacted when every closure blob is already local; the
        // first real HTTP attempt would fail fast against an unreachable
        // port, which the missing-blob test asserts on.
        SyncClient::new("http://127.0.0.1:9").expect("sync client")
    }

    fn apply_pulled_skill(store: &LocalStore, skill_id: &str, manifest_hash: &str) {
        let change = ChangeItem {
            sequence: 1,
            resource_type: "skill".to_owned(),
            resource_id: skill_id.to_owned(),
            resource_revision: 1,
            operation: "upsert".to_owned(),
            package_manifest_hash: Some(manifest_hash.to_owned()),
            metadata: json!({"name": "Pulled Skill", "slug": "pulled-skill"}),
        };
        store
            .apply_changes_page(&ChangesPage {
                protocol_version: 1,
                changes: vec![change],
                next_cursor: "v1.AAAAAQ".to_owned(),
                has_more: false,
            })
            .expect("apply page");
    }

    fn captured_snapshot(
        blobs: &crate::blob_store::BlobStore,
    ) -> (tempfile::TempDir, String, String) {
        let source = tempfile::tempdir().expect("source dir");
        std::fs::write(
            source.path().join("SKILL.md"),
            "# Hydrated Skill\n\nmaterialized from the server\n",
        )
        .expect("write skill.md");
        let reference = capture_workspace(
            blobs,
            source.path(),
            crate::skill_snapshot::SnapshotPolicy::default(),
        )
        .expect("capture");
        (
            source,
            reference.manifest_hash.clone(),
            reference.manifest_hash,
        )
    }

    #[test]
    fn pulled_skill_hydrates_into_workspace_and_promotes_record() {
        let temp = tempfile::tempdir().expect("tempdir");
        let blobs =
            crate::blob_store::BlobStore::open(temp.path().join("blobs")).expect("blob store");
        let workspaces =
            WorkspaceStore::open(temp.path().join("workspaces")).expect("workspace store");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("store");

        let (_source, manifest_hash, _) = captured_snapshot(&blobs);
        apply_pulled_skill(&store, "remote-1", &manifest_hash);

        let outcome = hydrate_skill(&offline_client(), &store, &blobs, &workspaces, "remote-1")
            .expect("hydrate");

        assert!(outcome.workspace_created);
        assert_eq!(outcome.blobs_downloaded, 0, "closure was already local");
        let skill = store.get_skill("remote-1").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::Synced);
        assert_eq!(skill.workspace_path, outcome.workspace_path);
        let entrypoint = outcome.workspace_path.join("SKILL.md");
        let content = std::fs::read_to_string(&entrypoint).expect("read entrypoint");
        assert!(content.contains("materialized from the server"));
    }

    #[test]
    fn hydrate_refuses_dirty_skill_without_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let blobs =
            crate::blob_store::BlobStore::open(temp.path().join("blobs")).expect("blob store");
        let workspaces =
            WorkspaceStore::open(temp.path().join("workspaces")).expect("workspace store");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("store");

        let (_source, manifest_hash, _) = captured_snapshot(&blobs);
        apply_pulled_skill(&store, "remote-2", &manifest_hash);
        store
            .lock_connection()
            .expect("conn")
            .execute(
                "UPDATE local_skills SET sync_state = 'dirty' WHERE id = 'remote-2'",
                [],
            )
            .expect("dirty");

        assert!(matches!(
            hydrate_skill(&offline_client(), &store, &blobs, &workspaces, "remote-2",),
            Err(HydrationError::LocalStore(LocalStoreError::InvalidInput(_)))
        ));
    }

    #[test]
    fn missing_closure_blob_fails_and_keeps_remote_only_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let blobs =
            crate::blob_store::BlobStore::open(temp.path().join("blobs")).expect("blob store");
        let workspaces =
            WorkspaceStore::open(temp.path().join("workspaces")).expect("workspace store");
        let store = LocalStore::open(temp.path().join("skillhive.db")).expect("store");

        // The pull feed delivered metadata whose manifest was never
        // downloaded; hydration must fail cleanly and keep the record
        // remote_only so a later pull can complete it.
        apply_pulled_skill(&store, "remote-3", "sha256:missing0manifest");

        let result = hydrate_skill(&offline_client(), &store, &blobs, &workspaces, "remote-3");
        assert!(result.is_err(), "unreachable server must fail the pull");
        let skill = store.get_skill("remote-3").expect("read").expect("skill");
        assert_eq!(skill.sync_state, SkillSyncState::RemoteOnly);
        assert!(workspaces.get("remote-3").expect("workspace").is_none());
    }
}
