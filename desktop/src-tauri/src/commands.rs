//! Tauri commands — a thin glue layer over `mc-core`. Every command maps a UI
//! request to a core call and serialises the result; long operations stream
//! progress / logs back as Tauri events. No launcher logic lives here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mc_core::auth::{AccountStore, MsaClient, StoredAccount};
use mc_core::download::Downloader;
use mc_core::instance::Instance;
use mc_core::launch::{self, LaunchSpec};
use mc_core::modplatform::modrinth::ModrinthApi;
use mc_core::modplatform::ResourceKind;
use mc_core::types::{
    AccountKind, AccountSummary, GameRoot, InstanceSummary, ManifestVersion, Progress, ThemeConfig,
};
use mc_core::{auth, java, meta, paths, LAUNCHER_NAME, LAUNCHER_VERSION};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{oneshot, watch};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn data_dir() -> PathBuf {
    paths::resolve_data_dir(&exe_dir())
}

fn default_root() -> PathBuf {
    let roots = paths::discover_roots(&exe_dir(), &data_dir(), &custom_roots());
    roots
        .first()
        .map(|r| PathBuf::from(&r.path))
        .unwrap_or_else(|| data_dir().join(".minecraft"))
}

fn root_paths(root: &str) -> paths::GamePaths {
    if root.is_empty() {
        paths::GamePaths::new(default_root())
    } else {
        paths::GamePaths::new(PathBuf::from(root))
    }
}

/// 加载全局设置(损坏/缺失回退默认,绝不报错)。
fn settings_global() -> mc_core::settings::GlobalSettings {
    mc_core::settings::GlobalSettings::load(&data_dir()).unwrap_or_default()
}

/// 用户在设置里添加的自定义游戏根目录(让 `custom_roots` 设置真正参与发现)。
fn custom_roots() -> Vec<PathBuf> {
    settings_global().custom_roots.iter().map(PathBuf::from).collect()
}

/// 按全局设置构造下载器:并发数 + 镜像源(官方/BMCLAPI+McIM)都来自用户设置,
/// 让「下载源/并发」这些全局设置真正生效。
fn make_downloader() -> CmdResult<Downloader> {
    let s = settings_global();
    Ok(Downloader::new(s.concurrency.max(1)).map_err(err)?.with_mirror(s.mirror_resolver()))
}

// --- DTOs that differ from the core types --------------------------------

/// JavaInstall with the version flattened to a string (the core keeps it
/// structured; the UI only displays it).
#[derive(Serialize)]
pub struct JavaDto {
    pub path: String,
    pub version: String,
    pub is_64bit: bool,
    pub source: String,
}

// --- read-only queries ----------------------------------------------------

#[tauri::command]
pub fn list_roots() -> CmdResult<Vec<GameRoot>> {
    Ok(paths::discover_roots(&exe_dir(), &data_dir(), &custom_roots()))
}

#[tauri::command]
pub fn list_instances(root: String) -> CmdResult<Vec<InstanceSummary>> {
    Ok(mc_core::instance::list_instances(&root_paths(&root)))
}

/// 取实例的游戏目录绝对路径(供「打开游戏目录」用前端 shell.open 打开)。
#[tauri::command]
pub fn instance_dir(root: String, id: String) -> CmdResult<String> {
    let paths = root_paths(&root);
    let dir = Instance::new(&id, paths.root().to_path_buf()).dir();
    Ok(dir.to_string_lossy().to_string())
}

/// 取实例某个子目录的绝对路径并确保其存在(供「打开目录」用前端 shell.open 打开)。
/// sub = "mods" / "resourcepacks" / "shaderpacks" / "datapacks" / "saves" / "screenshots" / "config"。
#[tauri::command]
pub fn instance_subdir(root: String, id: String, sub: String) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dir = match sub.as_str() {
        "mods" => inst.mods_dir(),
        "resourcepacks" => inst.resourcepacks_dir(),
        "shaderpacks" => inst.shaderpacks_dir(),
        "datapacks" => inst.datapacks_dir(),
        "saves" => inst.saves_dir(),
        "screenshots" => inst.screenshots_dir(),
        "config" => inst.game_dir().join("config"),
        other => return Err(format!("未知子目录: {other}")),
    };
    // 目录可能尚未被游戏创建过;先建好,避免 shell.open 打开不存在的路径失败。
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建目录失败: {e}"))?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 删除一个实例。复用 mc-core 的 lifecycle::delete_instance:优先移入系统回收站
/// (可恢复),无 GUI 时回退永久删除;目录不存在视为已删除(幂等)。前端须先确认。
#[tauri::command]
pub fn delete_instance(root: String, id: String) -> CmdResult<()> {
    mc_core::instance::lifecycle::delete_instance(&root_paths(&root), &id).map_err(err)
}

/// 复制一个实例:整目录复制 src_id → 新实例(id 由 new_name 唯一化),并把新实例
/// 的 instance.json name 设为 new_name。返回新实例 id。
#[tauri::command]
pub fn copy_instance(root: String, src_id: String, new_name: String) -> CmdResult<String> {
    mc_core::instance::lifecycle::copy_instance_named(&root_paths(&root), &src_id, &new_name)
        .map_err(err)
}

/// 从零创建实例:装核心(原版或 + loader)→ 写命名实例。进度走 install://progress。
/// loader = "vanilla" / "fabric" / "quilt" / "forge" / "neoforge";forge/neoforge 需 loader_version。
#[tauri::command]
pub async fn create_instance(
    app: AppHandle,
    root: String,
    name: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
) -> CmdResult<String> {
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let loader_opt = match parse_loader_kind(&loader) {
        None | Some(mc_core::types::LoaderKind::Vanilla) => None,
        Some(kind) => Some((kind, loader_version.unwrap_or_default())),
    };
    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "install://progress", rx);
    // 新实例的默认内存/Java 取自全局设置,让设置页的「默认内存 / Java 路径」真正生效。
    let g = settings_global();
    mc_core::instance::lifecycle::create_instance(
        &dl,
        &paths,
        &name,
        &mc_version,
        loader_opt,
        g.default_memory_mb,
        g.java_path.clone(),
        Some(tx),
    )
    .await
    .map_err(err)
}

/// 读取某实例的配置(名字/内存/Java/JVM 参数/窗口…)。文件缺失返回默认值。
#[tauri::command]
pub fn get_instance_config(root: String, id: String) -> CmdResult<mc_core::instance::InstanceConfig> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    inst.load_config().map_err(err)
}

/// 写某实例的配置(原子写入 instance.json)。
#[tauri::command]
pub fn set_instance_config(
    root: String,
    id: String,
    config: mc_core::instance::InstanceConfig,
) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    inst.save_config(&config).map_err(err)
}

/// 把任意图片设为实例图标(拷贝到 `versions/<id>/icon.png`)。source 为本地文件绝对路径。
/// 之后 list_instances 会把它探测为 data URL 回传前端。
#[tauri::command]
pub fn set_instance_icon(root: String, id: String, source: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    inst.set_icon(std::path::Path::new(&source)).map_err(err)
}

/// 列出某实例 mods 目录里的 mod(含启停态)。
#[tauri::command]
pub fn instance_mods(root: String, id: String) -> CmdResult<Vec<mc_core::instance::ModInfo>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    Ok(mc_core::instance::list_mods(&inst))
}

/// 启用/停用一个 mod(改 `.jar` ↔ `.jar.disabled`)。file_name 为 list 返回的稳定标识。
#[tauri::command]
pub fn set_mod_enabled(root: String, id: String, file_name: String, enabled: bool) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::mods::set_mod_enabled(&inst, &file_name, enabled).map_err(err)
}

/// 删除一个 mod 文件。
#[tauri::command]
pub fn delete_mod(root: String, id: String, file_name: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::mods::delete_mod(&inst, &file_name).map_err(err)
}

/// 从 Modrinth 把一个 mod(及其必需依赖)装进实例。loader/mc_version 用于挑兼容版本。
/// 返回安装报告(已装 / 已满足 / 未解决依赖)。
#[tauri::command]
pub async fn install_mod(
    root: String,
    id: String,
    project: String,
    mc_version: String,
    loader: String,
) -> CmdResult<mc_core::instance::InstallReport> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dl = make_downloader()?;
    let api = ModrinthApi::new();
    mc_core::instance::install_mod(&api, &dl, &inst, &project, &mc_version, &loader, true)
        .await
        .map_err(err)
}

/// 显式选版安装的结果:落盘主文件 + (仅 mod)依赖解析摘要。
#[derive(Serialize)]
pub struct VersionInstallReport {
    /// 主文件落盘名。
    file: String,
    /// 新装入的 required 依赖数量(仅 mod;packs 恒为 0)。
    installed_deps: usize,
    /// 找不到兼容版本、未能解决的 required 依赖 project_id(仅 mod)。
    unresolved: Vec<String>,
}

/// 安装一个**指定版本**(by Modrinth version id)到实例对应位置。
/// target = "mod" / "resourcepack" / "shader" / "datapack"。
///
/// mod:在装入所选版本的同时解析它声明的 required 依赖(取各依赖最新兼容版本),与「装最新版」
/// 行为一致 —— 避免选版安装出一个缺前置、进不去游戏的孤立 jar。需要 `mc_version` + `loader`
/// 才能给依赖挑兼容版本;缺省时退回只装主文件。packs 不涉及依赖。
#[tauri::command]
pub async fn install_version_file(
    root: String,
    id: String,
    target: String,
    version_id: String,
    mc_version: Option<String>,
    loader: Option<String>,
    world: Option<String>,
) -> CmdResult<VersionInstallReport> {
    use mc_core::instance::PackKind;
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dl = make_downloader()?;
    let api = ModrinthApi::new();
    let v = api.get_version(&version_id).await.map_err(err)?;
    let w = world.as_deref();

    let pack_report = |file: String| VersionInstallReport {
        file,
        installed_deps: 0,
        unresolved: Vec::new(),
    };

    match target.as_str() {
        "mod" => match (mc_version.as_deref(), loader.as_deref()) {
            (Some(mc), Some(ld)) => {
                let report =
                    mc_core::instance::install_mod_version_with_deps(&api, &dl, &inst, &v, mc, ld)
                        .await
                        .map_err(err)?;
                // 主文件是 installed 里 project_id 为空的那条;其余即新装的依赖。
                let file = report
                    .installed
                    .iter()
                    .find(|m| m.project_id.is_empty())
                    .map(|m| m.file_name.clone())
                    .unwrap_or_default();
                let installed_deps =
                    report.installed.iter().filter(|m| !m.project_id.is_empty()).count();
                Ok(VersionInstallReport {
                    file,
                    installed_deps,
                    unresolved: report.unresolved,
                })
            }
            _ => mc_core::instance::install_mod_version(&inst, &dl, &v)
                .await
                .map(pack_report)
                .map_err(err),
        },
        "resourcepack" => {
            mc_core::instance::packs::install_pack_version(&inst, &dl, PackKind::ResourcePack, &v, None)
                .await
                .map(pack_report)
                .map_err(err)
        }
        "shader" => mc_core::instance::packs::install_pack_version(&inst, &dl, PackKind::Shader, &v, None)
            .await
            .map(pack_report)
            .map_err(err),
        "datapack" => mc_core::instance::packs::install_pack_version(&inst, &dl, PackKind::Datapack, &v, w)
            .await
            .map(pack_report)
            .map_err(err),
        other => Err(format!("不支持的安装目标: {other}")),
    }
}

/// 检查实例里已启用 mod 的更新(对每个 jar 的 sha1 问 Modrinth 当前 loader/版本下的最新版)。
#[tauri::command]
pub async fn check_mod_updates(
    root: String,
    id: String,
    mc_version: String,
    loader: String,
) -> CmdResult<Vec<mc_core::instance::ModUpdate>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let api = ModrinthApi::new();
    mc_core::instance::check_mod_updates(&api, &inst, &mc_version, &loader)
        .await
        .map_err(err)
}

/// 应用一个 mod 更新:下载新版本进 mods/ 并删掉旧文件。update 为 check_mod_updates 返回的条目。
#[tauri::command]
pub async fn apply_mod_update(
    root: String,
    id: String,
    update: mc_core::instance::ModUpdate,
) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dl = make_downloader()?;
    mc_core::instance::apply_mod_update(&inst, &dl, &update)
        .await
        .map_err(err)
}

/// 把一个本地文件拖拽导入实例:按 target 拷贝到对应子目录,返回落盘文件名。
/// target = "mod" / "resourcepack" / "shader" / "datapack"。
#[tauri::command]
pub fn import_local_resource(
    root: String,
    id: String,
    target: String,
    path: String,
    world: Option<String>,
) -> CmdResult<String> {
    use mc_core::instance::PackKind;
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let src = std::path::Path::new(&path);
    let w = world.as_deref();
    match target.as_str() {
        "mod" => mc_core::instance::mods::import_local_mod(&inst, src).map_err(err),
        "resourcepack" => {
            mc_core::instance::packs::import_local_pack(&inst, PackKind::ResourcePack, src, None).map_err(err)
        }
        "shader" => {
            mc_core::instance::packs::import_local_pack(&inst, PackKind::Shader, src, None).map_err(err)
        }
        "datapack" => {
            mc_core::instance::packs::import_local_pack(&inst, PackKind::Datapack, src, w).map_err(err)
        }
        other => Err(format!("不支持的导入目标: {other}")),
    }
}

/// 列出某实例下指定类型的包(资源包 / 光影 / 数据包),含启停态。
#[tauri::command]
pub fn instance_packs(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    world: Option<String>,
) -> CmdResult<Vec<mc_core::instance::PackInfo>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    Ok(mc_core::instance::list_packs(&inst, kind, world.as_deref()))
}

/// 启用/停用一个包(改 `.zip` ↔ `.zip.disabled`)。
#[tauri::command]
pub fn set_pack_enabled(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    file_name: String,
    enabled: bool,
    world: Option<String>,
) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::packs::set_pack_enabled(&inst, kind, &file_name, enabled, world.as_deref())
        .map_err(err)
}

/// 删除一个包(移入系统回收站,可找回)。
#[tauri::command]
pub fn delete_pack(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    file_name: String,
    world: Option<String>,
) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::packs::delete_pack(&inst, kind, &file_name, world.as_deref()).map_err(err)
}

/// 从 Modrinth 安装一个包到实例对应目录,返回落盘文件名。
#[tauri::command]
pub async fn install_pack(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    project: String,
    mc_version: String,
    world: Option<String>,
) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dl = make_downloader()?;
    let api = ModrinthApi::new();
    mc_core::instance::install_pack(&api, &dl, &inst, kind, &project, &mc_version, world.as_deref())
        .await
        .map_err(err)
}

/// 列出某实例的截图(仅元数据,按修改时间倒序)。
#[tauri::command]
pub fn instance_screenshots(
    root: String,
    id: String,
) -> CmdResult<Vec<mc_core::instance::ScreenshotInfo>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    Ok(mc_core::instance::list_screenshots(&inst))
}

/// 按需读取一张截图为 data URL(UI 滚动到哪张才取哪张)。
#[tauri::command]
pub fn read_screenshot(root: String, id: String, file_name: String) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::read_screenshot(&inst, &file_name).map_err(err)
}

/// 删除一张截图(移入系统回收站,可找回)。
#[tauri::command]
pub fn delete_screenshot(root: String, id: String, file_name: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::screenshots::delete_screenshot(&inst, &file_name).map_err(err)
}

/// 列出某实例的存档世界(名字/模式/大小/上次游玩…)。
#[tauri::command]
pub fn instance_worlds(root: String, id: String) -> CmdResult<Vec<mc_core::instance::WorldInfo>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    Ok(mc_core::instance::list_worlds(&inst))
}

/// 删除一个存档世界(移入系统回收站,可找回)。
#[tauri::command]
pub fn delete_world(root: String, id: String, folder: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::world::delete_world(&inst, &folder).map_err(err)
}

/// 把一个存档打成 zip 备份到 dest_path(完整 .zip 文件路径,由 UI 的另存为对话框给出),
/// 返回写出的 zip 绝对路径。
#[tauri::command]
pub fn backup_world(root: String, id: String, folder: String, dest_path: String) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::world::backup_world(&inst, &folder, std::path::Path::new(&dest_path))
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(err)
}

/// 从一个 .zip 导入世界到实例 saves/,返回新世界文件夹名。zip 内需含 level.dat。
#[tauri::command]
pub fn import_world_zip(root: String, id: String, path: String) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::import_world_zip(&inst, std::path::Path::new(&path)).map_err(err)
}

/// 重命名存档的显示名(改 level.dat 的 LevelName,不改文件夹名)。
#[tauri::command]
pub fn rename_world(root: String, id: String, folder: String, new_name: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::world::rename_world(&inst, &folder, &new_name).map_err(err)
}

#[tauri::command]
pub async fn list_versions(snapshot: bool) -> CmdResult<Vec<ManifestVersion>> {
    let dl = make_downloader()?;
    let all = meta::fetch_manifest(&dl).await.map_err(err)?;
    Ok(if snapshot {
        all
    } else {
        all.into_iter()
            .filter(|v| matches!(v.kind, mc_core::types::ReleaseKind::Release))
            .collect()
    })
}

#[tauri::command]
pub fn list_accounts() -> CmdResult<Vec<AccountSummary>> {
    let store = AccountStore::load(data_dir().join("accounts.json")).map_err(err)?;
    Ok(store.list())
}

// --- accounts: Microsoft login + management ------------------------------

fn accounts_path() -> PathBuf {
    data_dir().join("accounts.json")
}

/// Persist a freshly built account, make it the selected one, and return its
/// summary. Shared by Microsoft and offline login.
fn store_and_select(account: StoredAccount) -> CmdResult<AccountSummary> {
    let _ = paths::ensure_dir(&data_dir());
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    let uuid = account.uuid.clone();
    store.add(account);
    store.select(&uuid).map_err(err)?;
    store.save().map_err(err)?;
    store
        .list()
        .into_iter()
        .find(|a| a.uuid == uuid)
        .ok_or_else(|| "登录成功但未能读回账号".to_string())
}

/// Build the Microsoft auth client.
///
/// The Application (client) ID is a **public** identifier, not a secret — it is
/// meant to be shipped baked into the binary (device-code / public-client flow
/// uses no client secret). Resolution order:
///   1. runtime env `MC_MSA_CLIENT_ID` — dev override (`src-tauri/.env`);
///   2. compile-time `MC_MSA_CLIENT_ID` — baked into release builds
///      (`MC_MSA_CLIENT_ID=… cargo build --release`), so end users need no setup;
///   3. the vanilla legacy id — last resort (rejected by device-code: AADSTS700016).
fn msa_client() -> MsaClient {
    let runtime = std::env::var("MC_MSA_CLIENT_ID").ok();
    let baked = option_env!("MC_MSA_CLIENT_ID").map(str::to_string);
    match runtime
        .or(baked)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        Some(id) => MsaClient::with_client_id(id),
        None => MsaClient::new(),
    }
}

/// The device-code prompt shown to the user. `device_code` is the opaque handle
/// passed back to [`msa_login_poll`]; everything else is for display.
#[derive(Serialize)]
pub struct DeviceCodeDto {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// Step ① of Microsoft login: start the device-code flow. The UI shows
/// `user_code` and opens `verification_uri`, then calls [`msa_login_poll`].
#[tauri::command]
pub async fn msa_login_start() -> CmdResult<DeviceCodeDto> {
    let info = msa_client().device_code_start().await.map_err(err)?;
    Ok(DeviceCodeDto {
        user_code: info.user_code,
        verification_uri: info.verification_uri,
        device_code: info.device_code,
        interval: info.interval,
        expires_in: info.expires_in,
    })
}

/// Step ② of Microsoft login: block until the user finishes in the browser,
/// run the full Xbox → XSTS → Minecraft → profile chain, then persist and
/// select the resulting account.
#[tauri::command]
pub async fn msa_login_poll(device_code: String, interval: u64) -> CmdResult<AccountSummary> {
    let client = msa_client();
    let token = client.poll_token(&device_code, interval).await.map_err(err)?;
    let session = client.authenticate(&token.access_token).await.map_err(err)?;
    store_and_select(StoredAccount {
        kind: AccountKind::Microsoft,
        username: session.username,
        uuid: session.uuid,
        access_token: session.access_token,
        refresh_token: Some(token.refresh_token),
        xuid: session.xuid,
        user_type: session.user_type,
        owns_game: true,
        // Minecraft access token 约 24h 有效;到期前用 refresh_token 自动续期。
        expires_at: Some(mc_core::auth::now_unix() + 86_400),
    })
}

/// Add (or update) an offline account from a username and select it.
#[tauri::command]
pub fn add_offline_account(name: String) -> CmdResult<AccountSummary> {
    let name = name.trim();
    if name.is_empty() {
        return Err("用户名不能为空".to_string());
    }
    let session = auth::offline_session(name);
    store_and_select(StoredAccount {
        kind: AccountKind::Offline,
        username: session.username,
        uuid: session.uuid,
        access_token: session.access_token,
        refresh_token: None,
        xuid: session.xuid,
        user_type: session.user_type,
        owns_game: false,
        expires_at: None,
    })
}

/// Switch the active account.
#[tauri::command]
pub fn select_account(uuid: String) -> CmdResult<()> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    store.select(&uuid).map_err(err)?;
    store.save().map_err(err)
}

/// Remove an account by uuid.
#[tauri::command]
pub fn remove_account(uuid: String) -> CmdResult<()> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    store.remove(&uuid);
    store.save().map_err(err)
}

/// 显式刷新当前选中的微软账号的登录(免浏览器,用 refresh_token)。返回是否执行了续期。
/// 失败(refresh_token 失效)时返回错误,UI 据此提示重新登录。
#[tauri::command]
pub async fn refresh_account() -> CmdResult<bool> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    // 显式刷新:用极大 margin 强制对选中的微软账号尝试续期,不看剩余有效期。
    auth::refresh_selected_microsoft(&mut store, &msa_client(), i64::MAX / 2)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn detect_java() -> CmdResult<Vec<JavaDto>> {
    let installs = java::detect_all().await;
    Ok(installs
        .into_iter()
        .map(|j| JavaDto {
            path: j.path.to_string_lossy().into_owned(),
            version: j.version.to_string(),
            is_64bit: j.is_64bit,
            source: j.source,
        })
        .collect())
}

#[tauri::command]
pub async fn modrinth_search(
    query: String,
    kind: String,
    game_version: Option<String>,
    loader: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> CmdResult<Vec<mc_core::modplatform::SearchHit>> {
    let api = ModrinthApi::new();
    let rk = match kind.as_str() {
        "modpack" => ResourceKind::Modpack,
        "shader" => ResourceKind::Shader,
        "resourcepack" => ResourceKind::ResourcePack,
        "datapack" => ResourceKind::Datapack,
        _ => ResourceKind::Mod,
    };
    api.search(
        &query,
        rk,
        game_version.as_deref(),
        loader.as_deref(),
        limit.unwrap_or(30),
        offset.unwrap_or(0),
    )
    .await
    .map_err(err)
}

// --- theme persistence ----------------------------------------------------

fn theme_path() -> PathBuf {
    data_dir().join("theme.json")
}

#[tauri::command]
pub fn get_theme() -> CmdResult<ThemeConfig> {
    match std::fs::read_to_string(theme_path()) {
        Ok(s) => serde_json::from_str(&s).map_err(err),
        Err(_) => Ok(ThemeConfig::default()),
    }
}

#[tauri::command]
pub fn set_theme(cfg: ThemeConfig) -> CmdResult<()> {
    let _ = paths::ensure_dir(&data_dir());
    let s = serde_json::to_string_pretty(&cfg).map_err(err)?;
    std::fs::write(theme_path(), s).map_err(err)
}

// --- progress / log plumbing ---------------------------------------------

/// Forward a [`Progress`] watch channel to a Tauri event until it closes.
fn forward_progress(app: AppHandle, event: &'static str, mut rx: watch::Receiver<Progress>) {
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow().clone();
            let _ = app.emit(event, p);
        }
    });
}

#[tauri::command]
pub async fn install_version(app: AppHandle, root: String, id: String) -> CmdResult<()> {
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let manifest = meta::fetch_manifest(&dl).await.map_err(err)?;
    let entry = manifest
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| format!("版本 {id} 不在清单中"))?;

    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "install://progress", rx);
    launch::install_version(&dl, &paths, &entry, Some(tx))
        .await
        .map_err(err)
}

/// 运行中的游戏进程表:instance id → 给该进程 reaper 任务发「请停止」的一次性信号。
///
/// 进程自然退出时由 reaper 自己把条目移除;[`stop_instance`] 主动停止时把 sender 取出
/// 并发信号。用 `Arc` 包裹以便克隆进 `'static` 的后台任务里(自然退出后自我注销)。
#[derive(Clone, Default)]
pub struct RunningGames {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<()>>>>,
}

impl RunningGames {
    fn register(&self, id: String, kill: oneshot::Sender<()>) {
        self.inner.lock().unwrap().insert(id, kill);
    }
    fn unregister(&self, id: &str) {
        self.inner.lock().unwrap().remove(id);
    }
    fn is_running(&self, id: &str) -> bool {
        self.inner.lock().unwrap().contains_key(id)
    }
    /// 取出并移除某实例的停止信号(若在运行)。
    fn take(&self, id: &str) -> Option<oneshot::Sender<()>> {
        self.inner.lock().unwrap().remove(id)
    }
    fn ids(&self) -> Vec<String> {
        self.inner.lock().unwrap().keys().cloned().collect()
    }
}

/// 启动一个实例。进程被登记进 [`RunningGames`];生命周期通过事件回传 UI:
/// 进度走 `launch://progress`,日志走 `game://log`,**真正 spawn 成功**后发
/// `game://started { id }`,进程退出后发 `game://exit { id, code, success, reason }`
/// (非零退出会跑崩溃诊断,把人话原因 + 建议一并带回)。
///
/// 同一实例已在运行时直接拒绝,避免重复开多个 JVM。
#[tauri::command]
pub async fn launch_instance(
    app: AppHandle,
    state: State<'_, RunningGames>,
    root: String,
    id: String,
    name: String,
    online: bool,
) -> CmdResult<()> {
    if state.is_running(&id) {
        return Err(format!("实例「{id}」已经在运行了"));
    }

    let paths = root_paths(&root);
    let dl = make_downloader()?;

    // 选中的微软账号若(接近)过期,先用 refresh_token 免浏览器续期(best-effort:
    // 失败就用现有 token 继续启动,不阻断游戏)。
    let accounts_path = data_dir().join("accounts.json");
    if let Ok(mut store) = AccountStore::load(&accounts_path) {
        let _ = auth::refresh_selected_microsoft(&mut store, &msa_client(), 600).await;
    }

    // Prefer the selected stored account; fall back to an offline session.
    let session = AccountStore::load(&accounts_path)
        .ok()
        .and_then(|s| s.selected_session())
        .unwrap_or_else(|| auth::offline_session(&name));

    // 是否联网修复文件:选了正版账号就联网(启动前补齐/修复缺损文件),离线账号走纯离线。
    // 离线 session 由 auth::offline_session 固定写入 access_token = "0" 标识。UI 传入的
    // online 作为下限,这样三个入口(Home/Library/经典)行为一致,不再因为某个入口硬编码
    // online=false 而跳过文件修复、导致残缺实例启动后神秘崩溃。
    let is_offline = session.access_token == "0" || session.access_token.is_empty();
    let online = online || !is_offline;

    let spec = LaunchSpec {
        instance: Instance::new(&id, paths.root().to_path_buf()),
        session,
        java_path: None,
        launcher_name: LAUNCHER_NAME.to_string(),
        launcher_version: LAUNCHER_VERSION.to_string(),
        online,
        runtimes_dir: Some(data_dir().join("java")),
        global_java_path: settings_global().java_path.filter(|p| !p.is_empty()).map(PathBuf::from),
    };

    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app.clone(), "launch://progress", rx);

    let mut child = launch::launch(spec, &dl, Some(tx)).await.map_err(err)?;

    // 滚动保留最近若干行输出,供进程退出后的崩溃诊断使用(崩溃原因多在 stderr 末尾)。
    let log_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Stream the game's stdout/stderr as log events (also drains the pipes so the
    // child never blocks on a full buffer).
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        let app = app.clone();
        let tail = log_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                push_tail(&tail, &line);
                let _ = app.emit("game://log", GameLog { line, level: "info" });
            }
        });
    }
    if let Some(e) = child.stderr.take() {
        let app = app.clone();
        let tail = log_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                push_tail(&tail, &line);
                let _ = app.emit("game://log", GameLog { line, level: "error" });
            }
        });
    }

    // 登记进程 + 通知 UI「真的起来了」(成功提示应以此为准,而非第一行日志)。
    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    state.register(id.clone(), kill_tx);
    let _ = app.emit("game://started", GameStarted { id: id.clone() });

    // 后台 reaper:等待自然退出或停止信号,reap 进程,注销登记,并回传退出/崩溃信息。
    let registry = state.inner_handle();
    tokio::spawn(async move {
        let status = tokio::select! {
            s = child.wait() => s.ok(),
            _ = kill_rx => {
                let _ = child.start_kill();
                child.wait().await.ok()
            }
        };
        registry.unregister(&id);

        let code = status.and_then(|s| s.code());
        let success = status.map(|s| s.success()).unwrap_or(false);
        let analysis = if success {
            None
        } else {
            let tail = log_tail.lock().unwrap().join("\n");
            mc_core::diagnostics::analyze_exit(code.unwrap_or(-1), &tail)
        };
        let (reason, suggestions) = match analysis {
            Some(a) => (Some(a.reason), a.suggestions),
            None => (None, Vec::new()),
        };
        let _ = app.emit(
            "game://exit",
            GameExit { id, code, success, reason, suggestions },
        );
    });

    Ok(())
}

/// 停止一个正在运行的实例(向其 reaper 发停止信号;reaper 杀进程并广播 `game://exit`)。
/// 实例不在运行时为 no-op。
#[tauri::command]
pub fn stop_instance(state: State<'_, RunningGames>, id: String) -> CmdResult<()> {
    if let Some(kill) = state.take(&id) {
        let _ = kill.send(());
    }
    Ok(())
}

/// 当前正在运行的实例 id 列表(供 UI 挂载时同步运行态)。
#[tauri::command]
pub fn running_instances(state: State<'_, RunningGames>) -> CmdResult<Vec<String>> {
    Ok(state.ids())
}

impl RunningGames {
    /// 克隆出可移动进 `'static` 后台任务的句柄(共享同一张表)。
    fn inner_handle(&self) -> RunningGames {
        self.clone()
    }
}

/// 把一行输出追加进滚动日志尾部,封顶 400 行,避免长会话无限增长。
fn push_tail(tail: &Arc<Mutex<Vec<String>>>, line: &str) {
    let mut t = tail.lock().unwrap();
    t.push(line.to_string());
    if t.len() > 400 {
        let overflow = t.len() - 400;
        t.drain(0..overflow);
    }
}

#[derive(Serialize, Clone)]
struct GameLog {
    line: String,
    level: &'static str,
}

#[derive(Serialize, Clone)]
struct GameStarted {
    id: String,
}

#[derive(Serialize, Clone)]
struct GameExit {
    id: String,
    /// 进程退出码(被信号杀死时可能为 `None`)。
    code: Option<i32>,
    success: bool,
    /// 非零退出时的人话崩溃原因(诊断命中才有)。
    reason: Option<String>,
    /// 崩溃诊断给出的可执行建议(可能为空)。
    suggestions: Vec<String>,
}

/// Diagnostic: the webview reports boot/errors here so they surface in stderr
/// (readable from the launch log even when we can't see the window).
#[tauri::command]
pub fn log_boot(msg: String) {
    eprintln!("[webview] {msg}");
}

// --- modpack import / export (thin glue over mc_core::modpack) ---------------

/// 一个 blocked 文件(CurseForge 作者禁第三方分发)的 UI 视图:需用户手动下载。
#[derive(Serialize)]
pub struct BlockedFileDto {
    pub name: String,
    pub website_url: String,
    pub target_dir: String,
    pub required: bool,
}

/// `import_modpack` 的返回:建好的实例 id + 需手动处理的 blocked 文件 + 跳过的可选文件。
#[derive(Serialize)]
pub struct ImportOutcomeDto {
    pub instance_id: String,
    pub blocked: Vec<BlockedFileDto>,
    pub skipped_optional: Vec<String>,
}

/// 导入一个整合包(`.mrpack` / CurseForge zip / MultiMC / MCBBS,自动识别格式),
/// 建好实例并返回其 id。`blocked` 列出需用户手动下载的 CurseForge 文件。
#[tauri::command]
pub async fn import_modpack(
    root: String,
    path: String,
    instance_id: Option<String>,
) -> CmdResult<ImportOutcomeDto> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};
    use mc_core::modplatform::provider::ProviderRegistry;

    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let engine = ImportEngine::with_defaults(dl, ProviderRegistry::with_defaults());

    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = instance_id;

    let outcome = engine
        .import(ImportSource::LocalFile(PathBuf::from(path)), opts)
        .await
        .map_err(err)?;

    Ok(ImportOutcomeDto {
        instance_id: outcome.instance_id,
        blocked: outcome
            .blocked
            .into_iter()
            .map(|b| BlockedFileDto {
                name: b.name,
                website_url: b.website_url,
                target_dir: b.target_dir,
                required: b.required,
            })
            .collect(),
        skipped_optional: outcome.skipped_optional,
    })
}

/// 把字符串解析成 loader 家族(导出时把 loader 依赖写进索引)。
fn parse_loader_kind(s: &str) -> Option<mc_core::types::LoaderKind> {
    use mc_core::types::LoaderKind;
    Some(match s.to_ascii_lowercase().as_str() {
        "forge" => LoaderKind::Forge,
        "neoforge" => LoaderKind::NeoForge,
        "fabric" => LoaderKind::Fabric,
        "quilt" => LoaderKind::Quilt,
        "liteloader" => LoaderKind::LiteLoader,
        "optifine" => LoaderKind::OptiFine,
        "vanilla" => LoaderKind::Vanilla,
        _ => return None,
    })
}

/// 把实例导出为整合包。`target` ∈ `modrinth` | `curseforge` | `modlist`
/// (后者可 `modlist:md|json|csv|txt|html` 选子格式)。`dest` 非空时把产物移到该路径。
/// 返回最终文件路径。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn export_modpack(
    root: String,
    instance_id: String,
    target: String,
    dest: Option<String>,
    pack_name: String,
    pack_version: Option<String>,
    mc_version: String,
    loader: Option<String>,
    loader_version: Option<String>,
) -> CmdResult<String> {
    use mc_core::modpack::export::{
        CurseForgeExportTarget, ExportInput, ExportTarget, ModListExportTarget, ModListFormat,
        ModpackExporter, ModrinthExportTarget,
    };

    let paths = root_paths(&root);
    let inst = Instance::new(instance_id.as_str(), paths.root().to_path_buf());
    let game_root = inst.game_dir();

    // 选目标(局部变量延长生命周期,再取 &dyn)。
    let (kind, sub) = target.split_once(':').unwrap_or((target.as_str(), ""));
    let mr = ModrinthExportTarget::new();
    let cf = CurseForgeExportTarget::new();
    let ml = ModListExportTarget::new(match sub {
        "html" => ModListFormat::Html,
        "json" => ModListFormat::Json,
        "csv" => ModListFormat::Csv,
        "txt" => ModListFormat::PlainText,
        _ => ModListFormat::Markdown,
    });
    let target_ref: &dyn ExportTarget = match kind {
        "modrinth" => &mr,
        "curseforge" => &cf,
        "modlist" => &ml,
        other => return Err(format!("未知导出目标: {other}")),
    };

    let mut input = ExportInput::new(&game_root, pack_name, mc_version);
    input.pack_version = pack_version;
    if let (Some(k), Some(v)) = (loader.as_deref(), loader_version) {
        if let Some(lk) = parse_loader_kind(k) {
            // 实例的 loader_version 实为整段版本 id;导出依赖前提取裸构建号,
            // 否则导出的 Forge/NeoForge 整合包再导入时会匹配不到 loader。
            let build = mc_core::loader::clean_loader_version(&v, lk, &input.mc_version);
            input.loader = Some((lk, build));
        }
    }

    let exporter = ModpackExporter::with_defaults();
    let out = exporter
        .export(target_ref, input, &mut |_, _, _| {})
        .await
        .map_err(err)?;

    // 用户指定了目标路径就把产物移过去(跨盘则拷贝后删原件)。
    let final_path = match dest {
        Some(d) if !d.trim().is_empty() => {
            let d = PathBuf::from(d);
            if std::fs::rename(&out, &d).is_err() {
                std::fs::copy(&out, &d).map_err(err)?;
                let _ = std::fs::remove_file(&out);
            }
            d
        }
        _ => out,
    };
    Ok(final_path.to_string_lossy().into_owned())
}

/// 从 Modrinth 安装一个整合包:取该项目最新版本的 `.mrpack` 下载地址,经导入引擎
/// 下载 + 识别 + 安装(原版 + loader + mods + overrides)成一个可启动实例。
#[tauri::command]
pub async fn install_modrinth_modpack(
    root: String,
    project_id: String,
    instance_id: Option<String>,
) -> CmdResult<ImportOutcomeDto> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};
    use mc_core::modplatform::provider::ProviderRegistry;

    // 1) 取最新版本的 .mrpack 下载地址。
    let api = ModrinthApi::new();
    let versions = api.get_versions(&project_id, None, None).await.map_err(err)?;
    let version = versions
        .into_iter()
        .next()
        .ok_or_else(|| format!("整合包 {project_id} 没有可用版本"))?;
    let url = version
        .files
        .iter()
        .find(|f| f.filename.ends_with(".mrpack"))
        .or_else(|| version.primary_file())
        .ok_or_else(|| "该整合包版本没有可下载的 .mrpack 文件".to_string())?
        .url
        .clone();

    // 2) 从 URL 导入(引擎先下到临时文件,再识别格式 + 安装)。
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let engine = ImportEngine::with_defaults(dl, ProviderRegistry::with_defaults());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = instance_id;
    let outcome = engine.import(ImportSource::Url(url), opts).await.map_err(err)?;

    Ok(ImportOutcomeDto {
        instance_id: outcome.instance_id,
        blocked: outcome
            .blocked
            .into_iter()
            .map(|b| BlockedFileDto {
                name: b.name,
                website_url: b.website_url,
                target_dir: b.target_dir,
                required: b.required,
            })
            .collect(),
        skipped_optional: outcome.skipped_optional,
    })
}

impl From<mc_core::modpack::import::ImportOutcome> for ImportOutcomeDto {
    fn from(o: mc_core::modpack::import::ImportOutcome) -> Self {
        ImportOutcomeDto {
            instance_id: o.instance_id,
            blocked: o
                .blocked
                .into_iter()
                .map(|b| BlockedFileDto {
                    name: b.name,
                    website_url: b.website_url,
                    target_dir: b.target_dir,
                    required: b.required,
                })
                .collect(),
            skipped_optional: o.skipped_optional,
        }
    }
}

/// 列出一个 Modrinth 整合包项目的所有版本详情(详情页用:版本号/类型/MC/loader/
/// 发布时间/下载数/changelog + 该版本 .mrpack 地址)。
#[tauri::command]
pub async fn modrinth_versions(
    project_id: String,
) -> CmdResult<Vec<mc_core::modplatform::modrinth::VersionDetail>> {
    ModrinthApi::new().version_details(&project_id).await.map_err(err)
}

/// 取一个 Modrinth 项目的完整详情(简介标签页用:长描述正文 markdown + 画廊 +
/// 关注数 + 源码/issue/wiki/discord 等外部链接)。
#[tauri::command]
pub async fn modrinth_project(
    project_id: String,
) -> CmdResult<mc_core::modplatform::modrinth::ProjectDetail> {
    ModrinthApi::new().project_details(&project_id).await.map_err(err)
}

/// 从一个 `.mrpack` 直链安装整合包(详情页「安装此版本」用)。
#[tauri::command]
pub async fn install_modpack_url(
    root: String,
    url: String,
    instance_id: Option<String>,
) -> CmdResult<ImportOutcomeDto> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};
    use mc_core::modplatform::provider::ProviderRegistry;

    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let engine = ImportEngine::with_defaults(dl, ProviderRegistry::with_defaults());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = instance_id;
    let outcome = engine.import(ImportSource::Url(url), opts).await.map_err(err)?;
    Ok(outcome.into())
}

/// 读取全局设置(下载源/并发/默认内存/Java 路径/语言…)。缺失/损坏回退默认。
#[tauri::command]
pub fn get_settings() -> CmdResult<mc_core::settings::GlobalSettings> {
    mc_core::settings::GlobalSettings::load(&data_dir()).map_err(err)
}

/// 持久化全局设置(原子写 settings.json)。下载相关项下次构造下载器即生效。
#[tauri::command]
pub fn set_settings(settings: mc_core::settings::GlobalSettings) -> CmdResult<()> {
    settings.save(&data_dir()).map_err(err)
}
