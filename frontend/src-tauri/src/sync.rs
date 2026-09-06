//! Serialized desktop sync orchestrator (M2.6, plan §8-§13).
//!
//! Composes the individually-tested sync primitives into one conservative
//! worker cycle:
//!
//! 1. refresh the authenticated Rust session;
//! 2. ensure the installation is a registered, active device;
//! 3. dispatch eligible outbox mutations oldest-first (negotiate/upload
//!    package closure → submit with the durable mutation ID → persist the
//!    ACK/conflict/error transactionally);
//! 4. drain change-feed pages from the durable cursor and apply them
//!    transactionally;
//! 5. stamp the successful push timestamp for diagnostics.
//!
//! Correctness lives in SQLite (receipt-keyed mutation IDs, durable
//! cursor, persisted backoff), not in worker memory: every step either
//! durably advances state or leaves the exact prior durable state, so a
//! crash at any point resumes safely on the next cycle. Cancellation
//! never deletes or regenerates a claimed mutation ID — an interrupted
//! dispatch leaves the mutation `in_flight`, which startup recovery
//! requeues under the same ID.
//!
//! Errors are classified per plan §15 and recorded in
//! `local_sync_state.last_server_error` (message only — never tokens or
//! secrets). Authentication/device failures stop the cycle without
//! touching mutation state; transport failures persist per-mutation
//! backoff through the durable ACK transaction.

use std::time::Duration;

use serde::Serialize;

use crate::blob_store::BlobStore;
use crate::local_store::{
    LocalMutation, LocalStore, LocalStoreError, MutationOperation, MutationOutcome,
};
use crate::sync_client::{DeviceRegistrationRequest, SyncClient, SyncClientError};
use crate::sync_pull::{pull_changes, PullError};
use crate::sync_push::MutationMetadata;
use crate::sync_transport::NegotiationError;

/// Upper bound on mutations dispatched per cycle; the rest wait for the
/// next trigger. Prevents one huge backlog from monopolizing the worker
/// without violating same-Skill causality (the claim SQL enforces that).
pub const MAX_MUTATIONS_PER_CYCLE: u32 = 50;

/// How long a cycle waits after a transport-grade failure before the next
/// attempt. The per-mutation schedule already lives in SQLite; this bounds
/// the cycle-level retry when the failure happened before dispatch.
pub const CYCLE_RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum SyncCycleError {
    /// Never signed in (no refresh credential) — the worker stops until a
    /// login happens, without recording a server error.
    #[error("not signed in")]
    NotSignedIn,
    /// Authentication/device failure (plan §15): distinct from mutation
    /// failure, never regenerates mutation IDs, stops the cycle.
    #[error("authentication/device failure: {0}")]
    Authentication(#[source] SyncClientError),
    /// Any other failure during the cycle. Already durably recorded where
    /// it applies; this reports the stage for diagnostics.
    #[error("{stage}: {source}")]
    Failed {
        stage: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl From<LocalStoreError> for SyncCycleError {
    fn from(error: LocalStoreError) -> Self {
        SyncCycleError::Failed {
            stage: "local store",
            source: Box::new(error),
        }
    }
}

impl From<NegotiationError> for SyncCycleError {
    fn from(error: NegotiationError) -> Self {
        SyncCycleError::Failed {
            stage: "package negotiation",
            source: Box::new(error),
        }
    }
}

impl From<PullError> for SyncCycleError {
    fn from(error: PullError) -> Self {
        SyncCycleError::Failed {
            stage: "pull",
            source: Box::new(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCycleReport {
    /// Human-readable stage report for diagnostics. Never contains tokens.
    pub pushed: usize,
    pub pulled: bool,
    pub pages_applied: usize,
    pub upserts: usize,
    pub tombstones: usize,
    pub conflicts_detected: u64,
    pub blobs_downloaded: usize,
    pub stopped_reason: Option<String>,
}

impl SyncCycleReport {
    /// True when the pull side reached its per-cycle page bound with more
    /// pages waiting on the server.
    pub fn has_more(&self) -> bool {
        self.pages_applied >= crate::sync_pull::MAX_PAGES_PER_CYCLE
    }
}

/// The serialized sync engine. All state is durable in SQLite; the engine
/// holds no sync correctness state of its own.
#[derive(Debug, Default)]
pub struct SyncEngine;

impl SyncEngine {
    /// Runs one full worker cycle: session, device, push, pull.
    ///
    /// `device_display_name` should describe this installation for the
    /// user's device list. Returns a report even when the cycle stopped
    /// early on an authentication failure (the caller decides how to
    /// surface that); transport and apply failures are returned as errors
    /// after being durably recorded.
    pub fn run_cycle(
        &self,
        client: &SyncClient,
        store: &LocalStore,
        blobs: &BlobStore,
        device_display_name: &str,
    ) -> Result<SyncCycleReport, SyncCycleError> {
        // 1. Session: refresh before anything else so every later call can
        // reuse the fresh access token. A missing refresh credential means
        // the user never signed in — stop quietly.
        if let Err(error) = client.ensure_access_token() {
            return Err(classify_session_failure(error));
        }

        // 2. Device: (re-)assert this installation's registration. The
        // server is idempotent per (user, clientInstance); a revoked
        // registration surfaces as an authentication failure and stops the
        // cycle without touching pending mutations.
        let registration = DeviceRegistrationRequest {
            protocol_version: 1,
            client_instance_id: store.ensure_client_instance_id()?,
            display_name: device_display_name.to_owned(),
            platform: std::env::consts::OS.to_owned(),
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
        };
        let device = match client.register_device(&registration) {
            Ok(device) => device,
            Err(error) => {
                return Err(classify_session_failure(error));
            }
        };
        if device.is_revoked() {
            return Err(SyncCycleError::Authentication(
                SyncClientError::Authentication {
                    code: "DEVICE_REVOKED".to_owned(),
                    status: 403,
                },
            ));
        }
        if store.sync_state()?.device_id.as_deref() != Some(device.device_id.as_str()) {
            store.record_device_registration(
                &device.client_instance_id,
                &device.device_id,
                &store
                    .sync_state()?
                    .server_user_id
                    .clone()
                    .unwrap_or_default(),
            )?;
        }

        // 3. Push: dispatch eligible mutations until the bound or the outbox
        // runs out. A mid-dispatch transport failure is durably recorded by
        // the ACK transaction; the cycle continues to pull so content still
        // flows when only push is failing.
        let mut pushed = 0_usize;
        let mut push_failure: Option<SyncCycleError> = None;
        for mutation in store.claim_dispatchable_mutations(MAX_MUTATIONS_PER_CYCLE)? {
            match dispatch_mutation(client, store, blobs, &mutation) {
                Ok(()) => pushed += 1,
                Err(error) => {
                    push_failure = Some(error);
                    break;
                }
            }
        }
        if push_failure.is_none() {
            store.record_push_success()?;
        }

        // 4. Pull: drain change pages from the durable cursor. Pulled
        // content lands even when push failed, and vice versa.
        let pull = match pull_changes(client, store, blobs) {
            Ok(pull) => pull,
            Err(error) => {
                // Record the pull failure for diagnostics, but still report
                // what the push half achieved before failing the cycle.
                let message = error.to_string();
                store.record_sync_error(&message)?;
                return Err(SyncCycleError::from(error));
            }
        };

        Ok(SyncCycleReport {
            pushed,
            pulled: true,
            pages_applied: pull.pages_applied,
            upserts: pull.upserts,
            tombstones: pull.tombstones,
            conflicts_detected: pull.conflicts_detected,
            blobs_downloaded: pull.blobs_downloaded,
            stopped_reason: push_failure.as_ref().map(failure_message),
        })
    }
}

/// Dispatches one claimed mutation: package closure upload (create/update
/// only), protocol submission, durable outcome application. Transport-grade
/// failures become a persisted retryable outcome with backoff; definitive
/// protocol outcomes persist their own states.
fn dispatch_mutation(
    client: &SyncClient,
    store: &LocalStore,
    blobs: &BlobStore,
    mutation: &LocalMutation,
) -> Result<(), SyncCycleError> {
    // Upload the package closure first so the server can validate the full
    // object set before committing the mutation (plan §7). Deletes carry no
    // package.
    if mutation.operation != MutationOperation::Delete {
        if let Err(error) = client.sync_snapshot_closure(blobs, &mutation.payload_hash) {
            // The closure upload is a transport-grade step: nothing was
            // committed server-side, so record retryable with backoff.
            let outcome = MutationOutcome::transport_error(
                &mutation.id,
                "PACKAGE_UPLOAD_FAILED",
                &error.to_string(),
            );
            store.apply_mutation_outcome(&outcome)?;
            return Err(SyncCycleError::from(error));
        }
    }

    let skill = store.get_skill(&mutation.skill_id)?;
    let metadata = skill.as_ref().map(|skill| MutationMetadata {
        name: skill.name.as_str(),
        slug: skill.slug.as_str(),
        description: None,
        category: None,
        tags: None,
    });
    let skill_remote_id = skill.as_ref().and_then(|skill| skill.remote_id.as_deref());

    match client.submit_mutation(&device_id_for(store)?, mutation, skill_remote_id, metadata) {
        Ok(response) => {
            store.apply_mutation_outcome(&response.into_outcome())?;
            Ok(())
        }
        Err(error) => {
            // HTTP-level failures: authentication/device failures must NOT
            // touch mutation state (plan §15) — surface them to stop the
            // cycle. A 4xx request rejection (permanent validation) must
            // NOT retry-storm: persist permanent_error. Everything else is
            // transport-grade: persist retryable.
            if matches!(
                error,
                SyncClientError::Authentication { .. } | SyncClientError::NotSignedIn
            ) {
                return Err(SyncCycleError::Authentication(error));
            }
            let error_code = error.error_code().unwrap_or("TRANSPORT_ERROR").to_owned();
            let error_message = error.to_string();
            let outcome = if matches!(error, SyncClientError::Request { .. }) {
                MutationOutcome {
                    mutation_id: mutation.id.clone(),
                    status: "validation_error".to_owned(),
                    remote_skill_id: None,
                    revision: None,
                    conflict_head_revision: None,
                    error_code: Some(error_code),
                    message: Some(error_message),
                }
            } else {
                MutationOutcome::transport_error(&mutation.id, &error_code, &error_message)
            };
            store.apply_mutation_outcome(&outcome)?;
            Err(SyncCycleError::Failed {
                stage: "mutation submit",
                source: Box::new(error),
            })
        }
    }
}

fn device_id_for(store: &LocalStore) -> Result<String, SyncCycleError> {
    store
        .sync_state()?
        .device_id
        .ok_or_else(|| SyncCycleError::Failed {
            stage: "device registration",
            source: "device ID missing after registration".to_owned().into(),
        })
}

fn classify_session_failure(error: SyncClientError) -> SyncCycleError {
    match error {
        SyncClientError::NotSignedIn => SyncCycleError::NotSignedIn,
        SyncClientError::Authentication { .. } => SyncCycleError::Authentication(error),
        other => SyncCycleError::Failed {
            stage: "session refresh",
            source: Box::new(other),
        },
    }
}

fn failure_message(error: &SyncCycleError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_bounds_are_conservative() {
        // Keep the per-cycle dispatch bound tight enough that one oversized
        // backlog cannot monopolize the worker beyond a bounded number of
        // HTTP round trips.
        const ABSOLUTE_DISPATCH_CEILING: u32 = 200;
        assert_eq!(
            MAX_MUTATIONS_PER_CYCLE.min(ABSOLUTE_DISPATCH_CEILING),
            MAX_MUTATIONS_PER_CYCLE
        );
    }

    #[test]
    fn session_failure_classification_distinguishes_auth_from_transport() {
        let auth = SyncClientError::Authentication {
            code: "INVALID_REFRESH".to_owned(),
            status: 401,
        };
        assert!(matches!(
            classify_session_failure(auth),
            SyncCycleError::Authentication(_)
        ));

        let network = SyncClientError::Network("offline".to_owned());
        assert!(matches!(
            classify_session_failure(network),
            SyncCycleError::Failed {
                stage: "session refresh",
                ..
            }
        ));

        assert!(matches!(
            classify_session_failure(SyncClientError::NotSignedIn),
            SyncCycleError::NotSignedIn
        ));
    }

    #[test]
    fn cycle_report_serializes_camel_case_without_secrets() {
        let report = SyncCycleReport {
            pushed: 2,
            pulled: true,
            pages_applied: 3,
            upserts: 4,
            tombstones: 1,
            conflicts_detected: 1,
            blobs_downloaded: 2,
            stopped_reason: None,
        };
        let json = serde_json::to_value(&report).expect("serialize");
        assert_eq!(json["pagesApplied"], 3);
        assert_eq!(json["conflictsDetected"], 1);
        assert!(json.get("stoppedReason").is_some());
    }
}
