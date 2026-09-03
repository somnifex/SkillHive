pub mod agent;
pub mod blob_store;
pub mod credentials;
pub mod deployment;
pub mod local_store;
pub mod skill_snapshot;
pub mod sync;

use std::{path::PathBuf, sync::Mutex};

use agent::{AgentDiscoveryResult, AgentRegistry};
use blob_store::BlobStore;
use deployment::{DeploymentEngine, DeploymentRequest, RecoveryReport};
use local_store::{
    AgentProfileRecord, DeploymentState, LocalStore, LocalStoreHealth, RecordDeployment,
    SkillDeploymentRecord, SkillSyncState, UpsertAgentProfile,
};
use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Debug, Default)]
pub struct DesktopMutationCoordinator {
    lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DesktopStartupStatus {
    pub local_store: LocalStoreHealth,
    pub recovered_in_flight_mutations: u64,
    pub deployment_recovery: RecoveryReport,
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
pub struct DeploySkillToAgentRequest {
    pub skill_id: String,
    pub agent_profile_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploySkillToAgentResult {
    pub deployment: SkillDeploymentRecord,
    /// True only when filesystem + SQLite are correct but journal cleanup failed.
    /// Startup recovery will retry the cleanup without repeating deployment.
    pub recovery_pending: bool,
}

#[tauri::command]
fn local_store_health(store: tauri::State<'_, LocalStore>) -> Result<LocalStoreHealth, String> {
    store.health().map_err(|error| error.to_string())
}

#[tauri::command]
fn discover_agents(registry: tauri::State<'_, AgentRegistry>) -> Vec<AgentDiscoveryResult> {
    registry.discover_all()
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
        .list_agent_profiles()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|profile| profile.id == request.agent_profile_id)
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

    // Do not ACK the filesystem journal until this SQLite write succeeds. If the
    // process dies here, startup recovery replays this idempotent catalog write.
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
fn desktop_startup_status(
    status: tauri::State<'_, DesktopStartupStatus>,
) -> DesktopStartupStatus {
    status.inner().clone()
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
            let mut deployment_recovery = deployment.recover_incomplete()?;

            // Filesystem recovery may complete activation before SQLite knew the
            // final state. Replay the catalog write, then ACK the journal. Any
            // failure is diagnostic and leaves the journal intact for retry.
            for recovered in deployment_recovery.catalog_commits.clone() {
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

            let local_store = store.health()?;
            let registry = AgentRegistry::builtin();

            app.manage(store);
            app.manage(blobs);
            app.manage(deployment);
            app.manage(registry);
            app.manage(DesktopMutationCoordinator::default());
            app.manage(DesktopStartupStatus {
                local_store,
                recovered_in_flight_mutations,
                deployment_recovery,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            local_store_health,
            discover_agents,
            list_agent_profiles,
            save_agent_profile,
            deploy_skill_to_agent,
            desktop_startup_status
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SkillHive desktop application");
}
