pub mod agent;
pub mod blob_store;
pub mod cache_manager;
pub mod credentials;
pub mod deployment;
pub mod local_store;
pub mod skill_snapshot;
pub mod snapshot_verifier;
pub mod sync;
pub mod uninstall;

use std::{path::PathBuf, sync::Mutex};

use agent::{AgentDescriptor, AgentDiscoveryResult, AgentInstance, AgentKind, AgentRegistry};
use blob_store::BlobStore;
use cache_manager::{enforce_cache_budget, CacheEnforcementReport};
use deployment::{DeploymentEngine, DeploymentRequest, RecoveryReport};
use local_store::{
    AgentProfileRecord, CommitSkillEdit, DeploymentState, LocalCachePolicy, LocalMutation,
    LocalSkill, LocalStore, LocalStoreHealth, MutationOperation, RecordDeployment,
    SkillDeploymentRecord, SkillSyncState, UpsertAgentProfile,
};
use serde::{Deserialize, Serialize};
use skill_snapshot::{capture_workspace, SkillSnapshotRef, SnapshotPolicy};
use snapshot_verifier::verify_materialized_snapshot;
use tauri::Manager;
use uninstall::{UninstallEngine, UninstallRequest};

#[derive(Debug, Default)]
pub struct DesktopMutationCoordinator {
    lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallStartupReport {
    pub discovered: usize,
    pub rolled_back: usize,
    pub finalized: usize,
    pub cleaned_intents: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopStartupStatus {
    pub local_store: LocalStoreHealth,
    pub recovered_in_flight_mutations: u64,
    pub deployment_recovery: RecoveryReport,
    pub uninstall_recovery: UninstallStartupReport,
    pub agent_reconciliation_errors: Vec<String>,
    pub cache_enforcement: Option<CacheEnforcementReport>,
    pub cache_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAgentProfileRequest {
    pub id: String,
    pub descriptor_id: String,
    pub display_name: String,
    pub skill_root: PathBuf,
    pub enabled: bool,
    pub is_custom: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitLocalSkillWorkspaceRequest {
    pub skill_id: String,
    pub remote_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub workspace_path: PathBuf,
    pub base_revision: Option<i64>,
    pub operation: MutationOperation,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitLocalSkillWorkspaceResult {
    pub skill: LocalSkill,
    pub snapshot: SkillSnapshotRef,
    pub mutation: LocalMutation,
    pub cache_enforcement: Option<CacheEnforcementReport>,
    pub cache_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploySkillToAgentRequest {
    pub skill_id: String,
    pub agent_profile_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploySkillToAgentResult {
    pub deployment: SkillDeploymentRecord,
    pub recovery_pending: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallSkillFromAgentRequest {
    pub skill_id: String,
    pub agent_profile_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallSkillFromAgentResult {
    pub skill_id: String,
    pub agent_profile_id: String,
    pub target_existed: bool,
    /// True when SQLite is already correct but quarantine/journal cleanup must
    /// be retried by startup recovery.
    pub recovery_pending: bool,
}

#[tauri::command]
fn local_store_health(store: tauri::State<'_, LocalStore>) -> Result<LocalStoreHealth, String> {
    store.health().map_err(|error| error.to_string())
}

#[tauri::command]
fn discover_agents(
    coordinator: tauri::State<'_, DesktopMutationCoordinator>,
    store: tauri::State<'_, LocalStore>,
    registry: tauri::State<'_, AgentRegistry>,
) -> Result<Vec<AgentDiscoveryResult>, String> {
    let _guard = coordinator
        .lock
        .lock()
        .map_err(|_| "desktop mutation coordinator lock poisoned".to_owned())?;
    discover_and_reconcile_agents(&store, &registry).map_err(|error| error.to_string())
}

#[tauri::command]
fn list_agent_profiles(
    store: tauri::State<'_, LocalStore>,
) -> Result<Vec<AgentProfileRecord>, String> {
    store.list_agent_profiles().map_err(|error| error.to_string())
}

#[tauri::command]
fn save_agent_profile(
    coordinator: tauri::State<'_, DesktopMutationCoordinator>,
    store: tauri::State<'_, LocalStore>,
    request: SaveAgentProfileRequest,
) -> Result<AgentProfileRecord, String> {
    let _guard = coordinator
        .lock
        .lock()
        .map_err(|_| "desktop mutation coordinator lock poisoned".to_owned())?;
    store
        .upsert_agent_profile(UpsertAgentProfile {
            id: request.id,
            descriptor_id: request.descriptor_id,
            display_name: request.display_name,
            skill_root: request.skill_root,
            enabled: request.enabled,
            is_custom: request.is_custom,
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn commit_local_skill_workspace(
    coordinator: tauri::State<'_, DesktopMutationCoordinator>,
    store: tauri::State<'_, LocalStore>,
    blobs: tauri::State<'_, BlobStore>,
    request: CommitLocalSkillWorkspaceRequest,
) -> Result<CommitLocalSkillWorkspaceResult, String> {
    let _guard = coordinator
        .lock
        .lock()
        .map_err(|_| "desktop mutation coordinator lock poisoned".to_owned())?;

    if request.operation == MutationOperation::Delete {
        return Err("workspace commit does not accept delete; deletion uses the tombstone path".to_owned());
    }

    let snapshot = capture_workspace(&blobs, &request.workspace_path, SnapshotPolicy::default())
        .map_err(|error| error.to_string())?;
    let mutation = store
        .commit_skill_edit(CommitSkillEdit {
            skill_id: request.skill_id.clone(),
            remote_id: request.remote_id,
            name: request.name,
            slug: request.slug,
            workspace_path: request.workspace_path,
            blob_hash: snapshot.manifest_hash.clone(),
            base_revision: request.base_revision,
            operation: request.operation,
        })
        .map_err(|error| error.to_string())?;
    let skill = store
        .get_skill(&request.skill_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("skill disappeared after local commit: {}", request.skill_id))?;

    let (cache_enforcement, cache_error) = match enforce_cache_budget(&store, &blobs) {
        Ok(report) => (Some(report), None),
        Err(error) => (None, Some(error.to_string())),
    };

    Ok(CommitLocalSkillWorkspaceResult {
        skill,
        snapshot,
        mutation,
        cache_enforcement,
        cache_error,
    })
}

#[tauri::command]
fn deploy_skill_to_agent(
    coordinator: tauri::State<'_, DesktopMutationCoordinator>,
    store: tauri::State<'_, LocalStore>,
    blobs: tauri::State<'_, BlobStore>,
    deployment_engine: tauri::State<'_, DeploymentEngine>,
    request: DeploySkillToAgentRequest,
) -> Result<DeploySkillToAgentResult, String> {
    let _guard = coordinator
        .lock
        .lock()
        .map_err(|_| "desktop mutation coordinator lock poisoned".to_owned())?;

    let skill = store
        .get_skill(&request.skill_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("skill not found: {}", request.skill_id))?;
    if matches!(
        skill.sync_state,
        SkillSyncState::RemoteOnly | SkillSyncState::AccessRevoked | SkillSyncState::Corrupted
    ) {
        return Err(format!(
            "skill {} is not deployable in state {:?}",
            skill.id, skill.sync_state
        ));
    }

    let profile = store
        .get_agent_profile(&request.agent_profile_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("agent profile not found: {}", request.agent_profile_id))?;
    if !profile.enabled {
        return Err(format!("agent profile is disabled: {}", profile.id));
    }

    let filesystem_result = deployment_engine
        .deploy(
            &blobs,
            DeploymentRequest {
                skill_id: skill.id.clone(),
                agent_profile_id: profile.id.clone(),
                snapshot_hash: skill.current_blob_hash.clone(),
                skill_root: profile.skill_root.clone(),
                directory_name: skill.slug.clone(),
            },
        )
        .map_err(|error| error.to_string())?;

    verify_materialized_snapshot(
        &blobs,
        &filesystem_result.snapshot_hash,
        &filesystem_result.target_path,
    )
    .map_err(|error| error.to_string())?;

    let catalog = store
        .record_deployment(RecordDeployment {
            skill_id: filesystem_result.skill_id.clone(),
            agent_profile_id: filesystem_result.agent_profile_id.clone(),
            deployed_blob_hash: filesystem_result.snapshot_hash.clone(),
            target_path: filesystem_result.target_path.clone(),
            state: DeploymentState::Installed,
            last_error: None,
        })
        .map_err(|error| error.to_string())?;

    let recovery_pending = deployment_engine
        .acknowledge_catalog_commit(&filesystem_result.transaction_id)
        .is_err();

    Ok(DeploySkillToAgentResult {
        deployment: catalog,
        recovery_pending,
    })
}

#[tauri::command]
fn uninstall_skill_from_agent(
    coordinator: tauri::State<'_, DesktopMutationCoordinator>,
    store: tauri::State<'_, LocalStore>,
    uninstall_engine: tauri::State<'_, UninstallEngine>,
    request: UninstallSkillFromAgentRequest,
) -> Result<UninstallSkillFromAgentResult, String> {
    let _guard = coordinator
        .lock
        .lock()
        .map_err(|_| "desktop mutation coordinator lock poisoned".to_owned())?;

    let deployment = store
        .get_deployment(&request.skill_id, &request.agent_profile_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "deployment not found for skill {} and profile {}",
                request.skill_id, request.agent_profile_id
            )
        })?;
    let profile = store
        .get_agent_profile(&request.agent_profile_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("agent profile not found: {}", request.agent_profile_id))?;
    if deployment.target_path.parent() != Some(profile.skill_root.as_path()) {
        return Err("deployment catalog target no longer matches agent profile root".to_owned());
    }

    let filesystem = uninstall_engine
        .begin(UninstallRequest {
            skill_id: deployment.skill_id.clone(),
            agent_profile_id: deployment.agent_profile_id.clone(),
            skill_root: profile.skill_root,
            target_path: deployment.target_path.clone(),
        })
        .map_err(|error| error.to_string())?;

    // The guarded delete matches both path and snapshot hash. On any DB error,
    // leave the uninstall journal intact; startup reads the authoritative catalog
    // and chooses rollback vs finalize instead of guessing whether commit landed.
    store
        .remove_deployment_catalog(
            &deployment.skill_id,
            &deployment.agent_profile_id,
            &deployment.deployed_blob_hash,
            &deployment.target_path,
        )
        .map_err(|error| error.to_string())?;

    let recovery_pending = filesystem
        .transaction_id
        .as_deref()
        .is_some_and(|transaction_id| uninstall_engine.finalize(transaction_id).is_err());

    Ok(UninstallSkillFromAgentResult {
        skill_id: deployment.skill_id,
        agent_profile_id: deployment.agent_profile_id,
        target_existed: filesystem.target_existed,
        recovery_pending,
    })
}

#[tauri::command]
fn enforce_local_cache(
    coordinator: tauri::State<'_, DesktopMutationCoordinator>,
    store: tauri::State<'_, LocalStore>,
    blobs: tauri::State<'_, BlobStore>,
) -> Result<CacheEnforcementReport, String> {
    let _guard = coordinator
        .lock
        .lock()
        .map_err(|_| "desktop mutation coordinator lock poisoned".to_owned())?;
    enforce_cache_budget(&store, &blobs).map_err(|error| error.to_string())
}

#[tauri::command]
fn set_local_cache_policy(
    coordinator: tauri::State<'_, DesktopMutationCoordinator>,
    store: tauri::State<'_, LocalStore>,
    blobs: tauri::State<'_, BlobStore>,
    policy: LocalCachePolicy,
) -> Result<CacheEnforcementReport, String> {
    let _guard = coordinator
        .lock
        .lock()
        .map_err(|_| "desktop mutation coordinator lock poisoned".to_owned())?;
    store
        .set_cache_policy(policy)
        .map_err(|error| error.to_string())?;
    enforce_cache_budget(&store, &blobs).map_err(|error| error.to_string())
}

#[tauri::command]
fn desktop_startup_status(
    status: tauri::State<'_, DesktopStartupStatus>,
) -> DesktopStartupStatus {
    status.inner().clone()
}

fn discover_and_reconcile_agents(
    store: &LocalStore,
    registry: &AgentRegistry,
) -> Result<Vec<AgentDiscoveryResult>, local_store::LocalStoreError> {
    let mut results = registry.discover_all();

    for result in &mut results {
        for instance in result.instances.clone() {
            let existing = store.get_agent_profile(&instance.id)?;
            if existing.as_ref().is_some_and(|profile| profile.is_custom) {
                append_discovery_error(
                    result,
                    format!(
                        "detected built-in profile {} collides with a persisted custom profile",
                        instance.id
                    ),
                );
                continue;
            }

            let enabled = existing.map(|profile| profile.enabled).unwrap_or(true);
            if let Err(error) = store.upsert_agent_profile(UpsertAgentProfile {
                id: instance.id.clone(),
                descriptor_id: instance.descriptor_id.clone(),
                display_name: instance.display_name.clone(),
                skill_root: instance.skill_root.clone(),
                enabled,
                is_custom: false,
            }) {
                append_discovery_error(
                    result,
                    format!("failed to reconcile profile {}: {error}", instance.id),
                );
            }
        }
    }

    for profile in store
        .list_agent_profiles()?
        .into_iter()
        .filter(|profile| profile.is_custom)
    {
        let validation_error = if profile.skill_root.exists() && !profile.skill_root.is_dir() {
            Some(format!(
                "{} exists but is not a directory",
                profile.skill_root.display()
            ))
        } else {
            None
        };
        results.push(AgentDiscoveryResult {
            descriptor: AgentDescriptor {
                id: profile.descriptor_id.clone(),
                display_name: profile.display_name.clone(),
                kind: AgentKind::Custom,
            },
            instances: if validation_error.is_none() {
                vec![AgentInstance {
                    id: profile.id,
                    descriptor_id: profile.descriptor_id,
                    display_name: profile.display_name,
                    skill_root_exists: profile.skill_root.exists(),
                    skill_root: profile.skill_root,
                    enabled: profile.enabled,
                    detected: true,
                }]
            } else {
                Vec::new()
            },
            error: validation_error,
        });
    }

    Ok(results)
}

fn append_discovery_error(result: &mut AgentDiscoveryResult, error: String) {
    match &mut result.error {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&error);
        }
        None => result.error = Some(error),
    }
}

fn reconcile_uninstall_recovery(
    store: &LocalStore,
    engine: &UninstallEngine,
) -> Result<UninstallStartupReport, String> {
    let discovered = engine.recover_pending().map_err(|error| error.to_string())?;
    let mut report = UninstallStartupReport {
        discovered: discovered.pending.len(),
        rolled_back: 0,
        finalized: 0,
        cleaned_intents: discovered.cleaned_intents,
        failed: discovered.failed,
    };

    for pending in discovered.pending {
        let Some(transaction_id) = pending.transaction_id.as_deref() else {
            continue;
        };
        match store.get_deployment(&pending.skill_id, &pending.agent_profile_id) {
            Ok(Some(_)) => match engine.rollback(transaction_id) {
                Ok(()) => report.rolled_back += 1,
                Err(error) => report.failed.push(format!(
                    "uninstall rollback failed for {transaction_id}: {error}"
                )),
            },
            Ok(None) => match engine.finalize(transaction_id) {
                Ok(()) => report.finalized += 1,
                Err(error) => report.failed.push(format!(
                    "uninstall finalize failed for {transaction_id}: {error}"
                )),
            },
            Err(error) => report.failed.push(format!(
                "uninstall catalog lookup failed for {transaction_id}: {error}"
            )),
        }
    }
    Ok(report)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            let store = LocalStore::open(data_dir.join("skillhive.sqlite3"))?;
            let recovered_in_flight_mutations = store.recover_in_flight_mutations()?;
            let blobs = BlobStore::open(data_dir.join("blobs"))?;
            let deployment = DeploymentEngine::open(data_dir.join("deployment-journal"))?;
            let uninstall = UninstallEngine::open(data_dir.join("uninstall-journal"))?;
            let mut deployment_recovery = deployment.recover_incomplete()?;

            for recovered in deployment_recovery.catalog_commits.clone() {
                if let Err(error) = verify_materialized_snapshot(
                    &blobs,
                    &recovered.snapshot_hash,
                    &recovered.target_path,
                ) {
                    deployment_recovery.failed.push(format!(
                        "deployment {} recovered a target that does not match snapshot {}: {error}",
                        recovered.transaction_id, recovered.snapshot_hash
                    ));
                    continue;
                }

                let catalog_result = store.record_deployment(RecordDeployment {
                    skill_id: recovered.skill_id.clone(),
                    agent_profile_id: recovered.agent_profile_id.clone(),
                    deployed_blob_hash: recovered.snapshot_hash.clone(),
                    target_path: recovered.target_path.clone(),
                    state: DeploymentState::Installed,
                    last_error: None,
                });
                match catalog_result {
                    Ok(_) => {
                        if let Err(error) =
                            deployment.acknowledge_catalog_commit(&recovered.transaction_id)
                        {
                            deployment_recovery.failed.push(format!(
                                "catalog committed for {}, but deployment journal ACK failed: {error}",
                                recovered.transaction_id
                            ));
                        }
                    }
                    Err(error) => deployment_recovery.failed.push(format!(
                        "deployment catalog reconciliation failed for {}: {error}",
                        recovered.transaction_id
                    )),
                }
            }

            let uninstall_recovery = reconcile_uninstall_recovery(&store, &uninstall)
                .unwrap_or_else(|error| UninstallStartupReport {
                    discovered: 0,
                    rolled_back: 0,
                    finalized: 0,
                    cleaned_intents: 0,
                    failed: vec![error],
                });

            let registry = AgentRegistry::builtin();
            let agent_results = discover_and_reconcile_agents(&store, &registry)?;
            let agent_reconciliation_errors = agent_results
                .into_iter()
                .filter_map(|result| {
                    result
                        .error
                        .map(|error| format!("{}: {error}", result.descriptor.id))
                })
                .collect();

            let (cache_enforcement, cache_error) = match enforce_cache_budget(&store, &blobs) {
                Ok(report) => (Some(report), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let local_store = store.health()?;

            app.manage(store);
            app.manage(blobs);
            app.manage(deployment);
            app.manage(uninstall);
            app.manage(registry);
            app.manage(DesktopMutationCoordinator::default());
            app.manage(DesktopStartupStatus {
                local_store,
                recovered_in_flight_mutations,
                deployment_recovery,
                uninstall_recovery,
                agent_reconciliation_errors,
                cache_enforcement,
                cache_error,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            local_store_health,
            discover_agents,
            list_agent_profiles,
            save_agent_profile,
            commit_local_skill_workspace,
            deploy_skill_to_agent,
            uninstall_skill_from_agent,
            enforce_local_cache,
            set_local_cache_policy,
            desktop_startup_status
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SkillHive desktop application");
}
