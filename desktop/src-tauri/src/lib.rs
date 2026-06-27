//! Tauri application assembly. `main.rs` calls [`run`]; all the launcher logic
//! is in `mc-core`, reached through the thin commands in [`commands`].
//!
//! 命令与 DTO 经 **tauri-specta** 导出为 `desktop/src/ipc/bindings.ts`(debug 构建时刷新),
//! 前端类型/调用签名从此由 Rust 单一真相生成,杜绝手写 TS 与后端漂移。

mod commands;
mod gallery;
mod logging;

use tauri_specta::{collect_commands, Builder};

/// Construct the tauri-specta [`Builder`] holding every command + event type.
///
/// Extracted so the TS bindings can be regenerated **without launching the GUI**:
/// the `export` test below builds this and writes `src/ipc/bindings.ts`. The
/// debug-launch export in [`run`] uses the same builder, so both paths stay in
/// lock-step and bindings never drift from the registered command set.
pub fn specta_builder() -> Builder<tauri::Wry> {
    // tauri-specta:收集所有命令(同时承载类型),debug 下把 TS 绑定写回前端 ipc 目录。
    // u64/i64(下载数、时间戳、字节数)按 number 导出,与既有前端一致(量级在 JS 安全整数内)。
    Builder::<tauri::Wry>::new()
        .dangerously_cast_bigints_to_number()
        // 事件 payload 类型也纳入生成(emit/listen 机制不变,仅消除手写事件类型漂移)。
        .typ::<mc_types::Progress>()
        .typ::<commands::GameLog>()
        .typ::<commands::GameStarted>()
        .typ::<commands::GameExit>()
        .commands(collect_commands![
            commands::list_roots,
            commands::list_instances,
            commands::instance_dir,
            commands::instance_subdir,
            commands::reveal_path,
            commands::delete_instance,
            commands::copy_instance,
            commands::create_instance,
            commands::install_loader,
            commands::list_loader_versions,
            commands::get_instance_config,
            commands::set_instance_config,
            commands::set_instance_tags,
            commands::set_instance_icon,
            commands::backfill_instance_icon,
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
            commands::instance_servers,
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
            commands::skin_profile,
            commands::skin_upload,
            commands::skin_set_cape,
            commands::detect_java,
            commands::modrinth_search,
            commands::content_facets,
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
            commands::install_modpack,
            commands::modrinth_versions,
            commands::modrinth_project,
            commands::get_settings,
            commands::set_settings,
            commands::social_enabled,
            commands::log_boot,
            commands::client_log,
            commands::open_logs_dir,
            commands::read_log_tail,
            commands::fetch_news,
            commands::kobemc_signup,
            commands::kobemc_login,
            commands::kobemc_session,
            commands::kobemc_logout,
            commands::realm_list,
            commands::realm_get,
            commands::realm_create,
            commands::realm_join,
            commands::realm_begin,
            commands::realm_members,
            commands::realm_push_manifest,
            commands::realm_plan_sync,
            commands::realm_sync,
            commands::realm_invite,
            commands::realm_set_role,
            commands::realm_remove_member,
            commands::realm_leave,
            commands::realm_disband,
            commands::friend_set_username,
            commands::friend_search,
            commands::friend_request,
            commands::friend_list,
            commands::friend_requests,
            commands::friend_accept,
            commands::friend_decline,
            commands::friend_remove,
            commands::presence_heartbeat,
            commands::account_link_microsoft,
            commands::account_identities,
            commands::account_unlink,
            commands::check_modpack_updates,
            commands::apply_modpack_update,
            gallery::gallery_enabled,
            gallery::gallery_capture,
            gallery::gallery_build,
        ])
}

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

    let builder = specta_builder();

    // 路径在编译期锚定到 crate 目录(运行时 CWD 不定,相对路径会写错地方)。
    #[cfg(debug_assertions)]
    builder
        .export(
            specta_typescript::Typescript::default(),
            concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts"),
        )
        .expect("failed to export typescript bindings");

    tauri::Builder::default()
        .setup(|app| {
            #[cfg(target_os = "macos")]
            {
                use tauri::Manager;
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
                if let Some(win) = app.get_webview_window("main") {
                    let _ = apply_vibrancy(
                        &win,
                        NSVisualEffectMaterial::HudWindow,
                        Some(NSVisualEffectState::Active),
                        None,
                    );
                }
            }
            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;
                use window_vibrancy::apply_acrylic;
                if let Some(win) = app.get_webview_window("main") {
                    let _ = apply_acrylic(&win, Some((18, 18, 22, 160)));
                }
            }
            Ok(())
        })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::RunningGames::default())
        // Shared mc-server client — kobeMC auth session cookie persists across calls.
        .manage(commands::kobe_client())
        .invoke_handler(builder.invoke_handler())
        .run(tauri::generate_context!())
        .expect("error while running mc-launcher");
}

#[cfg(test)]
mod export {
    /// Regenerate `src/ipc/bindings.ts` from the registered commands without a
    /// GUI launch. Run via `cargo test -p mc-launcher export_bindings`; keeps the
    /// hand-mirror-free contract enforced in CI / headless dev.
    #[test]
    fn export_bindings() {
        super::specta_builder()
            .export(
                specta_typescript::Typescript::default(),
                concat!(env!("CARGO_MANIFEST_DIR"), "/../src/ipc/bindings.ts"),
            )
            .expect("failed to export typescript bindings");
    }
}
