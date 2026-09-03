mod agent;
mod credentials;
mod deployment;
mod local_store;
mod sync;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            // M0 intentionally keeps startup side-effect free. Local database,
            // credential, synchronization, and deployment services are wired in
            // subsequent milestones behind explicit initialization boundaries.
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run SkillHive desktop application");
}
