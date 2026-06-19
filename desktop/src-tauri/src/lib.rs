//! Tauri application assembly. `main.rs` calls [`run`]; all the launcher logic
//! is in `mc-core`, reached through the thin commands in [`commands`].

mod commands;

/// Build and run the Tauri application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_roots,
            commands::list_instances,
            commands::list_versions,
            commands::list_accounts,
            commands::detect_java,
            commands::modrinth_search,
            commands::get_theme,
            commands::set_theme,
            commands::install_version,
            commands::launch_instance,
            commands::log_boot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mc-launcher");
}
