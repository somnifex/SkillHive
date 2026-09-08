//! Mutation push over the sync protocol (M2.4 desktop, plan §8).
//!
//! One claimed outbox mutation becomes one `POST /api/v1/sync/mutations`
//! request carrying the durable mutation ID. The server's receipt lookup
//! makes replays single-effect; the client's only obligation is to never
//! regenerate the ID. Definitive logical outcomes are returned for
//! [`crate::local_store::outcomes`] to apply; transport-grade failures
//! become `transport_error` so the mutation stays retryable.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::local_store::{LocalMutation, MutationOperation};
use crate::sync_client::SyncClient;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationRequest<'a> {
    pub protocol_version: u32,
    pub device_id: &'a str,
    pub mutation_id: &'a str,
    pub operation: &'static str,
    pub client_skill_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_skill_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_manifest_hash: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MutationMetadata<'a>>,
}

#[derive(Debug, Serialize)]
pub struct MutationMetadata<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<&'a [String]>,
}

/// Decoded `SyncMutationResponse` (server uses camelCase aliases).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResponse {
    #[allow(dead_code)]
    pub protocol_version: u32,
    pub mutation_id: Uuid,
    pub status: String,
    #[serde(default)]
    pub result: Option<MutationResultPayload>,
    #[serde(default)]
    pub conflict: Option<ConflictHeadPayload>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationResultPayload {
    pub remote_skill_id: String,
    pub revision: i64,
    #[serde(default)]
    pub package_manifest_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictHeadPayload {
    #[allow(dead_code)]
    pub remote_skill_id: String,
    #[allow(dead_code)]
    pub revision: i64,
    pub package_manifest_hash: Option<String>,
}

impl MutationResponse {
    /// Builds the durable-outcome struct for `local_store::outcomes`.
    pub fn into_outcome(self) -> crate::local_store::MutationOutcome {
        crate::local_store::MutationOutcome {
            mutation_id: self.mutation_id.to_string(),
            status: self.status,
            remote_skill_id: self.result.as_ref().map(|r| r.remote_skill_id.clone()),
            revision: self.result.as_ref().map(|r| r.revision),
            conflict_head_revision: self.conflict.as_ref().map(|head| head.revision),
            conflict_package_manifest_hash: self
                .conflict
                .as_ref()
                .and_then(|head| head.package_manifest_hash.clone()),
            error_code: self.error_code,
            message: self.message,
        }
    }
}

impl SyncClient {
    /// Submits one outbox mutation with its stable mutation ID.
    ///
    /// `remote_skill_id` falls back to the Skill's current `remote_id` when
    /// the mutation row itself has no acknowledged remote ID yet (an update
    /// against content the device created earlier: the create mutation
    /// carries the ACK, the follow-up update may not). The server requires
    /// `remoteSkillId` on update/delete, so dropping it would be a 422.
    pub fn submit_mutation(
        &self,
        device_id: &str,
        mutation: &LocalMutation,
        skill_remote_id: Option<&str>,
        metadata: Option<MutationMetadata<'_>>,
    ) -> Result<MutationResponse, crate::sync_client::SyncClientError> {
        let operation: &'static str = match mutation.operation {
            MutationOperation::Create => "create",
            MutationOperation::Update => "update",
            MutationOperation::Delete => "delete",
        };
        let remote_skill_id = mutation
            .acknowledged_remote_id
            .as_deref()
            .or(skill_remote_id);
        let request = MutationRequest {
            protocol_version: 1,
            device_id,
            mutation_id: &mutation.id,
            operation,
            client_skill_id: &mutation.skill_id,
            remote_skill_id,
            base_revision: mutation.base_revision,
            package_manifest_hash: mutation.package_manifest_hash_of(),
            metadata,
        };
        self.post_json("/api/v1/sync/mutations", &request)
    }
}

impl LocalMutation {
    fn package_manifest_hash_of(&self) -> Option<&str> {
        // Delete mutations carry no package; create/update reference the
        // snapshot payload hash.
        match self.operation {
            MutationOperation::Delete => None,
            _ => Some(self.payload_hash.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_serializes_camel_case_with_all_fields() {
        let request = MutationRequest {
            protocol_version: 1,
            device_id: "device-1",
            mutation_id: "00000000-0000-4000-8000-000000000001",
            operation: "create",
            client_skill_id: "skill-1",
            remote_skill_id: None,
            base_revision: None,
            package_manifest_hash: Some("sha256:abc"),
            metadata: Some(MutationMetadata {
                name: "Skill",
                slug: "skill",
                description: Some("d"),
                category: Some("general"),
                tags: Some(&["a".to_owned()]),
            }),
        };
        let json = serde_json::to_value(&request).expect("serialize");
        assert_eq!(json["protocolVersion"], 1);
        assert_eq!(json["deviceId"], "device-1");
        assert_eq!(json["mutationId"], "00000000-0000-4000-8000-000000000001");
        assert_eq!(json["clientSkillId"], "skill-1");
        assert_eq!(json["packageManifestHash"], "sha256:abc");
        assert!(json.get("remoteSkillId").is_none());
        assert!(json.get("baseRevision").is_none());
        assert_eq!(json["metadata"]["name"], "Skill");
    }

    #[test]
    fn response_deserializes_and_decodes_outcome() {
        let body = serde_json::json!({
            "protocolVersion": 1,
            "mutationId": "00000000-0000-4000-8000-000000000001",
            "status": "acked",
            "result": {
                "remoteSkillId": "remote-1",
                "revision": 3,
                "packageManifestHash": "sha256:abc"
            }
        });
        let response: MutationResponse = serde_json::from_value(body).expect("deserialize");
        assert_eq!(response.status, "acked");
        let outcome = response.into_outcome();
        assert_eq!(outcome.remote_skill_id.as_deref(), Some("remote-1"));
        assert_eq!(outcome.revision, Some(3));
    }

    #[test]
    fn conflict_response_decodes_head_revision() {
        let body = serde_json::json!({
            "protocolVersion": 1,
            "mutationId": "00000000-0000-4000-8000-000000000001",
            "status": "conflict",
            "conflict": {
                "remoteSkillId": "remote-1",
                "revision": 7,
                "packageManifestHash": "sha256:remote",
                "metadata": {}
            }
        });
        let response: MutationResponse = serde_json::from_value(body).expect("deserialize");
        let outcome = response.into_outcome();
        assert_eq!(outcome.status, "conflict");
        assert_eq!(outcome.conflict_head_revision, Some(7));
        assert_eq!(
            outcome.conflict_package_manifest_hash.as_deref(),
            Some("sha256:remote")
        );
    }
}
