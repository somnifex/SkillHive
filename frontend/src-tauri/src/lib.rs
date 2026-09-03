pub mod agent;
pub mod blob_store;
pub mod credentials;
pub mod deployment;
pub mod local_store;
pub mod sync;

use blob_store::BlobStore;
use local_store::{LocalStore, LocalStoreHealth};
use tauri::Manager;

#[tauri::command]
fn local_store_health(store: tauri::State<'_, LocalStore>) -> Result<LocalStoreHealth, String> {
    store.health().map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // M1 introduces intentional local durability initialization only.
            // Agent deployment, credential access, and network synchronization
            // remain explicit operations and are not started implicitly here.
            let data_dir = app.path().app_local_data_dir()?;
            let store = LocalStore::open(data_dir.join("skillhive.sqlite3"))?;
            let blobs = BlobStore::open(data_dir.join("blobs"))?;
            app.manage(store);
            app.manage(blobs);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![local_store_health])
        .run(tauri::generate_context!())
        .expect("failed to run SkillHive desktop application");
}
