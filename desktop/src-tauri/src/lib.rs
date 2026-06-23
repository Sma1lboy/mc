//! Tauri application assembly. `main.rs` calls [`run`]; all the launcher logic
//! is in `mc-core`, reached through the thin commands in [`commands`].

mod commands;
mod logging;

/// Build and run the Tauri application.
pub fn run() {
    // 先装日志:全局目录 <data_dir>/logs,client 与 daemon 两路统一收集。守卫持有到 run 结束
    //(即进程退出),保证缓冲日志被刷盘。
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let data_dir = mc_core::paths::resolve_data_dir(&exe_dir);
    let logs_dir = mc_core::paths::logs_dir(&data_dir);
    let _log_guard = logging::init(&logs_dir);
    tracing::info!(target: "daemon", "mc-launcher 启动,日志目录 {}", logs_dir.display());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::RunningGames::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_roots,
            commands::list_instances,
            commands::instance_dir,
            commands::instance_subdir,
            commands::delete_instance,
            commands::copy_instance,
            commands::create_instance,
            commands::get_instance_config,
            commands::set_instance_config,
            commands::set_instance_icon,
            commands::instance_mods,
            commands::set_mod_enabled,
            commands::delete_mod,
            commands::install_mod,
            commands::install_version_file,
            commands::check_mod_updates,
            commands::apply_mod_update,
            commands::import_local_resource,
            commands::instance_packs,
            commands::set_pack_enabled,
            commands::delete_pack,
            commands::install_pack,
            commands::instance_screenshots,
            commands::read_screenshot,
            commands::delete_screenshot,
            commands::instance_worlds,
            commands::delete_world,
            commands::backup_world,
            commands::rename_world,
            commands::import_world_zip,
            commands::list_versions,
            commands::list_accounts,
            commands::msa_login_start,
            commands::msa_login_poll,
            commands::add_offline_account,
            commands::yggdrasil_login,
            commands::select_account,
            commands::remove_account,
            commands::refresh_account,
            commands::detect_java,
            commands::modrinth_search,
            commands::get_theme,
            commands::set_theme,
            commands::install_version,
            commands::launch_instance,
            commands::stop_instance,
            commands::running_instances,
            commands::import_modpack,
            commands::export_modpack,
            commands::install_modrinth_modpack,
            commands::install_modpack_url,
            commands::modrinth_versions,
            commands::modrinth_project,
            commands::get_settings,
            commands::set_settings,
            commands::log_boot,
            commands::client_log,
            commands::open_logs_dir,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mc-launcher");
}
