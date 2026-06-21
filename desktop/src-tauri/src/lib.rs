//! Tauri application assembly. `main.rs` calls [`run`]; all the launcher logic
//! is in `mc-core`, reached through the thin commands in [`commands`].

mod commands;

/// Build and run the Tauri application.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_roots,
            commands::list_instances,
            commands::instance_dir,
            commands::delete_instance,
            commands::create_instance,
            commands::get_instance_config,
            commands::set_instance_config,
            commands::set_instance_icon,
            commands::instance_mods,
            commands::set_mod_enabled,
            commands::delete_mod,
            commands::install_mod,
            commands::instance_packs,
            commands::set_pack_enabled,
            commands::delete_pack,
            commands::install_pack,
            commands::instance_worlds,
            commands::delete_world,
            commands::list_versions,
            commands::list_accounts,
            commands::msa_login_start,
            commands::msa_login_poll,
            commands::add_offline_account,
            commands::select_account,
            commands::remove_account,
            commands::detect_java,
            commands::modrinth_search,
            commands::get_theme,
            commands::set_theme,
            commands::install_version,
            commands::launch_instance,
            commands::import_modpack,
            commands::export_modpack,
            commands::install_modrinth_modpack,
            commands::install_modpack_url,
            commands::modrinth_versions,
            commands::modrinth_project,
            commands::get_settings,
            commands::set_settings,
            commands::log_boot,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mc-launcher");
}
