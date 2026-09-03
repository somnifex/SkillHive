pub mod agent;
pub mod blob_store;
pub mod credentials;
pub mod deployment;
pub mod local_store;
pub mod sync;

use agent::{AgentDiscoveryResult, AgentRegistry};
use blob_store::BlobStore;
use deployment::{DeploymentEngine, RecoveryReport};
use local_store::{LocalStore, LocalStoreHealth};
use serde::Serialize;
use tauri::Manager;

#[derive(Debug, Clone, Serialize)]
pub struct DesktopStartupStatus {
    pub local_store: LocalStoreHealth,
    pub recovered_in_flight_mutations: u64,
    pub deployment_recovery: RecoveryReport,
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
fn desktop_startup_status(
    status: tauri::State<'_, DesktopStartupStatus>,
) -> DesktopStartupStatus {
    status.inner().clone()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // Startup performs only local correctness work. It does not refresh
            // credentials or access the network. Recovery is deterministic from
            // durable local state left by interrupted operations.
            let data_dir = app.path().app_local_data_dir()?;
            let store = LocalStore::open(data_dir.join("skillhive.sqlite3"))?;
            let recovered_in_flight_mutations = store.recover_in_flight_mutations()?;
            let blobs = BlobStore::open(data_dir.join("blobs"))?;
            let deployment = DeploymentEngine::open(data_dir.join("deployment-journal"))?;
            let deployment_recovery = deployment.recover_incomplete()?;
            let local_store = store.health()?;
            let registry = AgentRegistry::builtin();

            app.manage(store);
            app.manage(blobs);
            app.manage(deployment);
            app.manage(registry);
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
            desktop_startup_status
        ])
        .run(tauri::generate_context!())
        .expect("failed to run SkillHive desktop application");
}
