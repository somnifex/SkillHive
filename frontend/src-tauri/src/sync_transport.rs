//! Package blob negotiation and verified upload (M2.4 desktop, plan §7).
//!
//! Before submitting a mutation the sync worker prepares the manifest
//! closure (manifest hash + every file blob), asks the server which
//! objects are missing, and streams only those objects to
//! `PUT /api/v1/sync/blobs/{hash}`. The server recomputes the digest while
//! consuming the stream, so a corrupt body is rejected before it becomes
//! addressable — mutation correctness never trusts the client's claim.

use serde::{Deserialize, Serialize};

use crate::blob_store::{BlobStore, BlobStoreError};
use crate::skill_snapshot::{read_manifest, SkillSnapshotFile};
use crate::sync_client::{SyncClient, SyncClientError};

/// Server limit: `MAX_BLOB_NEGOTIATION_ITEMS`.
pub const MAX_NEGOTIATION_ITEMS: usize = 1024;

#[derive(Debug, thiserror::Error)]
pub enum NegotiationError {
    #[error("manifest not found locally: {0}")]
    ManifestMissing(String),
    #[error("blob missing locally: {0}")]
    BlobMissing(String),
    #[error("blob store failure: {0}")]
    BlobStore(String),
    #[error("negotiation rejected: {0}")]
    Http(#[from] SyncClientError),
    #[error("snapshot declares too many objects for one negotiation")]
    TooManyObjects,
}

impl From<BlobStoreError> for NegotiationError {
    fn from(error: BlobStoreError) -> Self {
        match error {
            BlobStoreError::NotFound(hash) => Self::BlobMissing(hash),
            other => Self::BlobStore(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BlobDescriptor {
    pub hash: String,
    pub size_bytes: u64,
}

impl BlobDescriptor {
    fn from_snapshot_file(file: &SkillSnapshotFile) -> Self {
        Self {
            hash: file.blob_hash.clone(),
            size_bytes: file.size_bytes,
        }
    }
}

/// Wire models (server uses camelCase aliases via `to_camel`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MissingBlobsRequest<'a> {
    protocol_version: u32,
    objects: &'a [BlobDescriptor],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MissingBlobsResponse {
    #[allow(dead_code)]
    protocol_version: u32,
    missing: Vec<BlobDescriptor>,
}

/// Result of negotiating and uploading one snapshot closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosureSynced {
    pub manifest_hash: String,
    pub uploaded: usize,
}

/// Reads the local manifest and returns its closure: every file blob plus
/// the manifest blob itself (all content-addressed).
pub fn snapshot_closure(
    blobs: &BlobStore,
    manifest_hash: &str,
) -> Result<Vec<BlobDescriptor>, NegotiationError> {
    let manifest = read_manifest(blobs, manifest_hash)
        .map_err(|_| NegotiationError::ManifestMissing(manifest_hash.to_owned()))?;
    let mut descriptors: Vec<BlobDescriptor> = manifest
        .files
        .iter()
        .map(BlobDescriptor::from_snapshot_file)
        .collect();
    let manifest_bytes = blobs.read_bytes(manifest_hash)?;
    descriptors.push(BlobDescriptor {
        hash: manifest_hash.to_owned(),
        size_bytes: manifest_bytes.len() as u64,
    });
    descriptors.sort_by(|left, right| left.hash.cmp(&right.hash));
    descriptors.dedup_by(|left, right| left.hash == right.hash);
    Ok(descriptors)
}

impl SyncClient {
    /// Negotiates missing objects and uploads them. Returns how many blobs
    /// were uploaded (0 means the server already holds the full closure).
    pub fn sync_snapshot_closure(
        &self,
        blobs: &BlobStore,
        manifest_hash: &str,
    ) -> Result<ClosureSynced, NegotiationError> {
        let closure = snapshot_closure(blobs, manifest_hash)?;
        if closure.len() > MAX_NEGOTIATION_ITEMS {
            return Err(NegotiationError::TooManyObjects);
        }

        let response: MissingBlobsResponse = self.post_json(
            "/api/v1/sync/blobs/missing",
            &MissingBlobsRequest {
                protocol_version: 1,
                objects: &closure,
            },
        )?;

        let mut uploaded = 0_usize;
        for blob in &response.missing {
            let bytes = blobs.read_bytes(&blob.hash)?;
            self.put_octet_stream(&format!("/api/v1/sync/blobs/{}", blob.hash), &bytes)?;
            uploaded += 1;
        }
        Ok(ClosureSynced {
            manifest_hash: manifest_hash.to_owned(),
            uploaded,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_descriptor_serializes_camel_case() {
        let descriptor = BlobDescriptor {
            hash: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            size_bytes: 5,
        };
        let json = serde_json::to_value(&descriptor).expect("serialize");
        assert_eq!(json["sizeBytes"], 5);
        assert!(json.get("size_bytes").is_none());
    }

    #[test]
    fn missing_blobs_response_deserializes_camel_case() {
        let body = serde_json::json!({
            "protocolVersion": 1,
            "missing": [
                {"hash": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                 "sizeBytes": 3}
            ]
        });
        let response: MissingBlobsResponse = serde_json::from_value(body).expect("deserialize");
        assert_eq!(response.missing.len(), 1);
        assert_eq!(response.missing[0].size_bytes, 3);
    }
}
