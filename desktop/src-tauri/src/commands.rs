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

/// 解析最终生效的 CurseForge API key(设置 → 编译期 baked → 环境)。
/// 既给注册表决定是否注册 CurseForge,也给下载器附 `x-api-key` 让 CF CDN 直链可下。
fn cf_api_key() -> Option<String> {
    mc_core::modplatform::provider::resolve_cf_api_key(settings_global().cf_api_key.as_deref())
}

/// 按用户设置/环境构造 Provider 注册表:总有 Modrinth;解析出 CurseForge key 才注册 CurseForge。
/// 搜索 / 浏览安装 / 整合包导入导出共用同一份(让「设置里的 CF key」与环境 key 都生效)。
fn make_registry() -> mc_core::modplatform::provider::ProviderRegistry {
    mc_core::modplatform::provider::ProviderRegistry::with_defaults_keyed(cf_api_key())
}

/// 按平台标识取一个已注册的 provider;未注册(如无 CF key)时报清晰错误。
fn provider_or_err(
    reg: &mc_core::modplatform::provider::ProviderRegistry,
    id: mc_core::modplatform::ProviderId,
) -> CmdResult<std::sync::Arc<dyn mc_core::modplatform::provider::ResourceProvider>> {
    reg.get(id).ok_or_else(|| match id {
        mc_core::modplatform::ProviderId::CurseForge => "CurseForge 未配置 API Key".to_string(),
        mc_core::modplatform::ProviderId::Modrinth => "Modrinth 不可用".to_string(),
    })
}

/// 把前端传入的 provider 字符串(缺省 `modrinth`)映射成 [`ProviderId`]。
fn parse_provider(s: Option<&str>) -> CmdResult<mc_core::modplatform::ProviderId> {
    use mc_core::modplatform::ProviderId;
    match s.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("modrinth") {
        "modrinth" => Ok(ProviderId::Modrinth),
        "curseforge" => Ok(ProviderId::CurseForge),
        other => Err(format!("未知内容平台: {other}")),
    }
}

/// CurseForge 作者禁第三方分发时,平台不给 `downloadUrl`(映射后 `file.url` 为空串)。
/// 用与整合包导入相同的官网手动下载页拼法,给前端 BlockedFilesDialog 引导用户手动下载。
fn cf_blocked_dto(project_id: &str, file_id: &str, file_name: &str, target_dir: &str) -> BlockedFileDto {
    BlockedFileDto {
        name: file_name.to_string(),
        website_url: format!(
            "https://www.curseforge.com/api/v1/mods/{project_id}/files/{file_id}/download"
        ),
        target_dir: target_dir.to_string(),
        required: true,
    }
}

/// 按全局设置构造下载器:并发数 + 镜像源(官方/BMCLAPI+McIM)+ CurseForge key 都来自
/// 用户设置/环境,让「下载源/并发」这些全局设置真正生效、CF CDN 直链带上 `x-api-key`。
fn make_downloader() -> CmdResult<Downloader> {
    let s = settings_global();
    Ok(Downloader::new(s.concurrency.max(1))
        .map_err(err)?
        .with_mirror(s.mirror_resolver())
        .with_cf_api_key(cf_api_key()))
}

// --- DTOs that differ from the core types --------------------------------

/// JavaInstall with the version flattened to a string (the core keeps it
/// structured; the UI only displays it).
#[derive(Serialize, specta::Type)]
pub struct JavaDto {
    pub path: String,
    pub version: String,
    pub is_64bit: bool,
    pub source: String,
}

// --- read-only queries ----------------------------------------------------

#[tauri::command]
#[specta::specta]
pub fn list_roots() -> CmdResult<Vec<GameRoot>> {
    Ok(paths::discover_roots(&exe_dir(), &data_dir(), &custom_roots()))
}

#[tauri::command]
#[specta::specta]
pub fn list_instances(root: String) -> CmdResult<Vec<InstanceSummary>> {
    Ok(mc_core::instance::list_instances(&root_paths(&root)))
}

/// 取实例的游戏目录绝对路径(供「打开游戏目录」用前端 shell.open 打开)。
#[tauri::command]
#[specta::specta]
pub fn instance_dir(root: String, id: String) -> CmdResult<String> {
    let paths = root_paths(&root);
    let dir = Instance::new(&id, paths.root().to_path_buf()).dir();
    Ok(dir.to_string_lossy().to_string())
}

/// 用系统文件管理器打开一个路径(目录/文件)。直接调 OS,绕开 shell 插件只放行 URL 的作用域。
#[tauri::command]
#[specta::specta]
pub fn reveal_path(path: String) -> CmdResult<()> {
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(target_os = "windows")]
    let spawned = std::process::Command::new("explorer").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open").arg(&path).spawn();
    spawned.map(|_| ()).map_err(err)
}

/// 取实例某个子目录的绝对路径并确保其存在(供「打开目录」用前端 shell.open 打开)。
/// sub = "mods" / "resourcepacks" / "shaderpacks" / "datapacks" / "saves" / "screenshots" / "config"。
#[tauri::command]
#[specta::specta]
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
#[specta::specta]
pub fn delete_instance(root: String, id: String) -> CmdResult<()> {
    mc_core::instance::lifecycle::delete_instance(&root_paths(&root), &id).map_err(err)
}

/// 复制一个实例:整目录复制 src_id → 新实例(id 由 new_name 唯一化),并把新实例
/// 的 instance.json name 设为 new_name。返回新实例 id。
#[tauri::command]
#[specta::specta]
pub fn copy_instance(root: String, src_id: String, new_name: String) -> CmdResult<String> {
    mc_core::instance::lifecycle::copy_instance_named(&root_paths(&root), &src_id, &new_name)
        .map_err(err)
}

/// 从零创建实例:装核心(原版或 + loader)→ 写命名实例。进度走 install://progress。
/// loader = "vanilla" / "fabric" / "quilt" / "forge" / "neoforge";forge/neoforge 需 loader_version。
#[tauri::command]
#[specta::specta]
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

/// 给一个已存在的实例添加 / 切换 mod 加载器(core)。进度走 install://progress。
/// loader = "fabric" / "quilt" / "forge" / "neoforge"(拒绝 vanilla / 无效值);
/// forge/neoforge 需 loader_version。返回之后应使用的实例 id —— 多数情况与传入 id
/// 相同,但「实例目录本身就是裸原版」的退化情形会返回一个新 id(避免自环)。
#[tauri::command]
#[specta::specta]
pub async fn install_loader(
    app: AppHandle,
    root: String,
    id: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
) -> CmdResult<String> {
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let kind = match parse_loader_kind(&loader) {
        Some(mc_core::types::LoaderKind::Vanilla) | None => {
            return Err(format!("无效的加载器: {loader}"));
        }
        Some(kind) => kind,
    };
    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "install://progress", rx);
    mc_core::instance::lifecycle::add_loader(
        &dl,
        &paths,
        &id,
        &mc_version,
        (kind, loader_version.unwrap_or_default()),
        Some(tx),
    )
    .await
    .map_err(err)
}

/// 列出某 loader 在指定 MC 版本下的可用构建号(新建实例的版本选择器用)。
/// loader = "forge" / "neoforge" / "fabric" / "quilt";其它(vanilla 等)返回空。
/// 返回值按「新→旧」排序,前端默认选第一个。网络/解析失败时由前端回退到手填输入框。
#[tauri::command]
#[specta::specta]
pub async fn list_loader_versions(
    loader: String,
    mc_version: String,
) -> CmdResult<Vec<String>> {
    let kind = match parse_loader_kind(&loader) {
        Some(k) => k,
        None => return Ok(Vec::new()),
    };
    let dl = make_downloader()?;
    mc_core::loader::list_loader_versions(&dl, kind, &mc_version)
        .await
        .map_err(err)
}

/// 读取某实例的配置(名字/内存/Java/JVM 参数/窗口…)。文件缺失返回默认值。
#[tauri::command]
#[specta::specta]
pub fn get_instance_config(root: String, id: String) -> CmdResult<mc_core::instance::InstanceConfig> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    inst.load_config().map_err(err)
}

/// 写某实例的配置(原子写入 instance.json)。
#[tauri::command]
#[specta::specta]
pub fn set_instance_config(
    root: String,
    id: String,
    config: mc_core::instance::InstanceConfig,
) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    inst.save_config(&config).map_err(err)
}

/// 本机内存信息(供 UI 在内存滑块旁展示「系统内存 X GB」)。
#[derive(Serialize, specta::Type)]
pub struct SystemMemory {
    /// 物理内存总量(MiB)。探测失败时为 0。
    pub total_mb: u64,
}

/// 读取本机物理内存总量(MiB)。纯探测,不读实例。
#[tauri::command]
#[specta::specta]
pub fn system_memory() -> CmdResult<SystemMemory> {
    Ok(SystemMemory { total_mb: mc_core::system::system_total_mem_mb() })
}

/// 为某实例推荐一个最大堆内存(MiB):综合本机物理内存与该实例已装 mod 数量。
/// 纯启发式(见 [`mc_core::system::suggest_memory_mb`]),按需读取一次 mod 列表 + 系统内存。
#[tauri::command]
#[specta::specta]
pub fn suggest_instance_memory(root: String, id: String) -> CmdResult<u32> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let mod_count = mc_core::instance::list_mods(&inst).len();
    let total = mc_core::system::system_total_mem_mb();
    Ok(mc_core::system::suggest_memory_mb(total, mod_count))
}

/// 设置某实例的标签:加载配置 → 规范化(去空白、去空项、去重、保序)→ 写回。
/// 自由格式标签,供库页分组 / 按标签筛选用。
#[tauri::command]
#[specta::specta]
pub fn set_instance_tags(root: String, id: String, tags: Vec<String>) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let mut config = inst.load_config().map_err(err)?;
    let mut seen = std::collections::HashSet::new();
    config.tags = tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty() && seen.insert(t.clone()))
        .collect();
    inst.save_config(&config).map_err(err)
}

/// 把任意图片设为实例图标(拷贝到 `versions/<id>/icon.png`)。source 为本地文件绝对路径。
/// 之后 list_instances 会把它探测为 data URL 回传前端。
#[tauri::command]
#[specta::specta]
pub fn set_instance_icon(root: String, id: String, source: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    inst.set_icon(std::path::Path::new(&source)).map_err(err)
}

/// 给整合包来源的实例补齐图标:实例还没有本地 `icon.png` 时,下载 `icon_url` 写入。
/// 早于「安装即存图标」特性安装的实例本地没图标,详情页发现缺失时调用一次,让侧栏/首页/详情
/// 统一显示整合包真实 logo,而不再退回默认像素占位。返回 `true` 表示这次补上了。
/// 已有图标或下载失败都安静返回 `false`(图标纯展示性,失败不打断任何流程)。
#[tauri::command]
#[specta::specta]
pub async fn backfill_instance_icon(root: String, id: String, icon_url: String) -> CmdResult<bool> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    if inst.has_icon() || icon_url.trim().is_empty() {
        return Ok(false);
    }
    let Ok(dl) = make_downloader() else { return Ok(false) };
    match dl.get_bytes(&icon_url).await {
        Ok(bytes) => Ok(inst.set_icon_bytes(&bytes).is_ok()),
        Err(_) => Ok(false),
    }
}

/// 列出某实例 mods 目录里的 mod(含启停态)。
#[tauri::command]
#[specta::specta]
pub fn instance_mods(root: String, id: String) -> CmdResult<Vec<mc_core::instance::ModInfo>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    Ok(mc_core::instance::list_mods(&inst))
}

/// 启用/停用一个 mod(改 `.jar` ↔ `.jar.disabled`)。file_name 为 list 返回的稳定标识。
#[tauri::command]
#[specta::specta]
pub fn set_mod_enabled(root: String, id: String, file_name: String, enabled: bool) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::mods::set_mod_enabled(&inst, &file_name, enabled).map_err(err)
}

/// 删除一个 mod 文件。
#[tauri::command]
#[specta::specta]
pub fn delete_mod(root: String, id: String, file_name: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::mods::delete_mod(&inst, &file_name).map_err(err)
}

/// 「装最新版」mod 的结果:沿用核心的依赖解析报告,再带上需手动下载的 blocked 文件
/// (CurseForge 作者禁第三方分发时)。`blocked` 非空时前端弹 BlockedFilesDialog 引导手动下。
#[derive(Default, Serialize, specta::Type)]
pub struct ModInstallReport {
    pub installed: Vec<mc_core::instance::install_mod::InstalledMod>,
    pub satisfied: Vec<String>,
    pub unresolved: Vec<String>,
    #[serde(default)]
    pub incompatible: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<BlockedFileDto>,
}

impl From<mc_core::instance::InstallReport> for ModInstallReport {
    fn from(r: mc_core::instance::InstallReport) -> Self {
        ModInstallReport {
            installed: r.installed,
            satisfied: r.satisfied,
            unresolved: r.unresolved,
            incompatible: r.incompatible,
            blocked: Vec::new(),
        }
    }
}

/// 把一个 mod 的最新兼容版本装进实例。`provider` 缺省 `modrinth`:
/// - Modrinth 走核心的「装最新版 + 解析 required 依赖」路径。
/// - CurseForge 经 provider 取最新兼容版本直接落盘(CF 文件级不带依赖,故不解析);
///   遇作者禁分发的文件经 `blocked` 回传,前端走手动下载流而非假装成功。
#[tauri::command]
#[specta::specta]
pub async fn install_mod(
    root: String,
    id: String,
    project: String,
    mc_version: String,
    loader: String,
    provider: Option<String>,
) -> CmdResult<ModInstallReport> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dl = make_downloader()?;
    match parse_provider(provider.as_deref())? {
        mc_core::modplatform::ProviderId::Modrinth => {
            let api = ModrinthApi::new();
            mc_core::instance::install_mod(&api, &dl, &inst, &project, &mc_version, &loader, true)
                .await
                .map(ModInstallReport::from)
                .map_err(err)
        }
        id @ mc_core::modplatform::ProviderId::CurseForge => {
            let p = provider_or_err(&make_registry(), id)?;
            let versions = p
                .list_versions(&project, Some(&mc_version), Some(&loader))
                .await
                .map_err(err)?;
            let v = mc_core::instance::install_mod::pick_version(&versions, &mc_version, &loader)
                .or_else(|| versions.first())
                .ok_or_else(|| format!("项目 {project} 没有兼容 {mc_version}/{loader} 的版本"))?;
            let Some(file) = v.primary_file() else {
                return Err(format!("版本 {} 没有可下载文件", v.id));
            };
            if file.url.is_empty() {
                return Ok(ModInstallReport {
                    blocked: vec![cf_blocked_dto(&project, &v.id, &file.filename, "mods")],
                    ..Default::default()
                });
            }
            let file_name = mc_core::instance::install_mod::install_mod_version(&inst, &dl, v)
                .await
                .map_err(err)?;
            Ok(ModInstallReport {
                installed: vec![mc_core::instance::install_mod::InstalledMod {
                    project_id: project,
                    file_name,
                }],
                ..Default::default()
            })
        }
    }
}

/// 显式选版安装的结果:落盘主文件 + (仅 mod)依赖解析摘要 + 需手动下载的 blocked 文件。
#[derive(Default, Serialize, specta::Type)]
pub struct VersionInstallReport {
    /// 主文件落盘名(被 blocked 时为空)。
    file: String,
    /// 新装入的 required 依赖数量(仅 mod;packs 恒为 0)。
    installed_deps: usize,
    /// 找不到兼容版本、未能解决的 required 依赖 project_id(仅 mod)。
    unresolved: Vec<String>,
    /// 所装版本声明为不兼容的项目 project_id(冲突;仅 mod)。
    #[serde(default)]
    incompatible: Vec<String>,
    /// CurseForge 作者禁第三方分发时需用户手动下载的文件;非空时前端弹 BlockedFilesDialog。
    #[serde(default)]
    blocked: Vec<BlockedFileDto>,
}

/// `target` → 包类型 + blocked 引导用的落盘目录名。
fn pack_kind_for(target: &str) -> CmdResult<(mc_core::instance::PackKind, &'static str)> {
    use mc_core::instance::PackKind;
    Ok(match target {
        "resourcepack" => (PackKind::ResourcePack, "resourcepacks"),
        "shader" => (PackKind::Shader, "shaderpacks"),
        "datapack" => (PackKind::Datapack, "datapacks"),
        other => return Err(format!("不支持的安装目标: {other}")),
    })
}

/// 安装一个**指定版本**(by version id)到实例对应位置。`provider` 缺省 `modrinth`,
/// `project` 是该版本所属项目 id(CurseForge 经 `get_files_bulk` 反查需要,Modrinth 可空)。
/// target = "mod" / "resourcepack" / "shader" / "datapack"。
///
/// mod(仅 Modrinth):在装入所选版本的同时解析它声明的 required 依赖(取各依赖最新兼容版本),
/// 与「装最新版」一致 —— 避免选版安装出一个缺前置、进不去游戏的孤立 jar。需要 `mc_version` +
/// `loader` 才能给依赖挑兼容版本;缺省时退回只装主文件。packs 与 CurseForge 不涉及依赖。
/// CurseForge 作者禁分发的文件经 `blocked` 回传,前端走手动下载流而非假装成功。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub async fn install_version_file(
    root: String,
    id: String,
    target: String,
    version_id: String,
    mc_version: Option<String>,
    loader: Option<String>,
    world: Option<String>,
    provider: Option<String>,
    project: Option<String>,
) -> CmdResult<VersionInstallReport> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dl = make_downloader()?;
    let w = world.as_deref();

    let pack_report = |file: String| VersionInstallReport { file, ..Default::default() };

    let (v, is_modrinth) = match parse_provider(provider.as_deref())? {
        mc_core::modplatform::ProviderId::Modrinth => {
            (ModrinthApi::new().get_version(&version_id).await.map_err(err)?, true)
        }
        id @ mc_core::modplatform::ProviderId::CurseForge => {
            let project = project
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "CurseForge 安装需要项目 id".to_string())?;
            let p = provider_or_err(&make_registry(), id)?;
            let mut files = p
                .get_files_bulk(&[(project.to_string(), version_id.clone())])
                .await
                .map_err(err)?;
            let resolved = files
                .pop()
                .ok_or_else(|| format!("CurseForge 版本 {version_id} 不存在"))?;
            // 禁第三方分发 → url 为空串:走与导入相同的 blocked 流,绝不假装成功。
            if resolved.file.url.is_empty() {
                let dir = if target == "mod" { "mods" } else { pack_kind_for(&target)?.1 };
                return Ok(VersionInstallReport {
                    blocked: vec![cf_blocked_dto(project, &version_id, &resolved.file.filename, dir)],
                    ..Default::default()
                });
            }
            // 把解析出的文件包成一个单文件 ProjectVersion 喂给与平台无关的落盘函数。
            let v = mc_core::modplatform::ProjectVersion {
                id: resolved.version_id,
                name: resolved.file.filename.clone(),
                version_number: resolved.file.filename.clone(),
                game_versions: Vec::new(),
                loaders: Vec::new(),
                files: vec![resolved.file],
                dependencies: Vec::new(),
            };
            (v, false)
        }
    };

    match target.as_str() {
        // CurseForge 文件级不带依赖模型 → 只装主文件;Modrinth 且给了 mc/loader 才解析依赖。
        "mod" => match (is_modrinth, mc_version.as_deref(), loader.as_deref()) {
            (true, Some(mc), Some(ld)) => {
                let api = ModrinthApi::new();
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
                    incompatible: report.incompatible,
                    ..Default::default()
                })
            }
            _ => mc_core::instance::install_mod_version(&inst, &dl, &v)
                .await
                .map(pack_report)
                .map_err(err),
        },
        other => {
            let (kind, _) = pack_kind_for(other)?;
            mc_core::instance::packs::install_pack_version(&inst, &dl, kind, &v, w)
                .await
                .map(pack_report)
                .map_err(err)
        }
    }
}

/// 检查实例里已启用 mod 的更新(对每个 jar 的 sha1 问 Modrinth 当前 loader/版本下的最新版)。
#[tauri::command]
#[specta::specta]
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
#[specta::specta]
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
#[specta::specta]
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
#[specta::specta]
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
#[specta::specta]
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
#[specta::specta]
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

/// 安装一个包(资源包 / 光影 / 数据包)的最新兼容版本到实例对应目录。`provider` 缺省
/// `modrinth`。返回落盘文件名;CurseForge 作者禁分发的文件经 `blocked` 回传(file 为空),
/// 前端走手动下载流而非假装成功。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub async fn install_pack(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    project: String,
    mc_version: String,
    world: Option<String>,
    provider: Option<String>,
) -> CmdResult<VersionInstallReport> {
    use mc_core::instance::PackKind;
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let dl = make_downloader()?;
    let w = world.as_deref();
    match parse_provider(provider.as_deref())? {
        mc_core::modplatform::ProviderId::Modrinth => {
            let api = ModrinthApi::new();
            mc_core::instance::install_pack(&api, &dl, &inst, kind, &project, &mc_version, w)
                .await
                .map(|file| VersionInstallReport { file, ..Default::default() })
                .map_err(err)
        }
        pid @ mc_core::modplatform::ProviderId::CurseForge => {
            let p = provider_or_err(&make_registry(), pid)?;
            let versions = p.list_versions(&project, Some(&mc_version), None).await.map_err(err)?;
            let v = versions
                .iter()
                .find(|v| v.game_versions.iter().any(|g| g == mc_version.as_str()))
                .or_else(|| versions.first())
                .ok_or_else(|| format!("项目 {project} 没有兼容 {mc_version} 的版本"))?;
            let Some(file) = v.primary_file() else {
                return Err(format!("版本 {} 没有可下载文件", v.id));
            };
            if file.url.is_empty() {
                let dir = match kind {
                    PackKind::ResourcePack => "resourcepacks",
                    PackKind::Shader => "shaderpacks",
                    PackKind::Datapack => "datapacks",
                };
                return Ok(VersionInstallReport {
                    blocked: vec![cf_blocked_dto(&project, &v.id, &file.filename, dir)],
                    ..Default::default()
                });
            }
            mc_core::instance::packs::install_pack_version(&inst, &dl, kind, v, w)
                .await
                .map(|file| VersionInstallReport { file, ..Default::default() })
                .map_err(err)
        }
    }
}

/// 列出某实例的截图(仅元数据,按修改时间倒序)。
#[tauri::command]
#[specta::specta]
pub fn instance_screenshots(
    root: String,
    id: String,
) -> CmdResult<Vec<mc_core::instance::ScreenshotInfo>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    Ok(mc_core::instance::list_screenshots(&inst))
}

/// 按需读取一张截图为 data URL(UI 滚动到哪张才取哪张)。
#[tauri::command]
#[specta::specta]
pub fn read_screenshot(root: String, id: String, file_name: String) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::read_screenshot(&inst, &file_name).map_err(err)
}

/// 删除一张截图(移入系统回收站,可找回)。
#[tauri::command]
#[specta::specta]
pub fn delete_screenshot(root: String, id: String, file_name: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::screenshots::delete_screenshot(&inst, &file_name).map_err(err)
}

/// 列出某实例的存档世界(名字/模式/大小/上次游玩…)。
#[tauri::command]
#[specta::specta]
pub fn instance_worlds(root: String, id: String) -> CmdResult<Vec<mc_core::instance::WorldInfo>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    Ok(mc_core::instance::list_worlds(&inst))
}

/// 列出某实例已保存的多人服务器(读 game_dir/servers.dat;文件不存在 → 空表)。
#[tauri::command]
#[specta::specta]
pub fn instance_servers(root: String, id: String) -> CmdResult<Vec<mc_core::instance::SavedServer>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::read_servers(&inst.game_dir()).map_err(err)
}

/// 删除一个存档世界(移入系统回收站,可找回)。
#[tauri::command]
#[specta::specta]
pub fn delete_world(root: String, id: String, folder: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::world::delete_world(&inst, &folder).map_err(err)
}

/// 把一个存档打成 zip 备份到 dest_path(完整 .zip 文件路径,由 UI 的另存为对话框给出),
/// 返回写出的 zip 绝对路径。
#[tauri::command]
#[specta::specta]
pub fn backup_world(root: String, id: String, folder: String, dest_path: String) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::world::backup_world(&inst, &folder, std::path::Path::new(&dest_path))
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(err)
}

/// 从一个 .zip 导入世界到实例 saves/,返回新世界文件夹名。zip 内需含 level.dat。
#[tauri::command]
#[specta::specta]
pub fn import_world_zip(root: String, id: String, path: String) -> CmdResult<String> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::import_world_zip(&inst, std::path::Path::new(&path)).map_err(err)
}

/// 重命名存档的显示名(改 level.dat 的 LevelName,不改文件夹名)。
#[tauri::command]
#[specta::specta]
pub fn rename_world(root: String, id: String, folder: String, new_name: String) -> CmdResult<()> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    mc_core::instance::world::rename_world(&inst, &folder, &new_name).map_err(err)
}

#[tauri::command]
#[specta::specta]
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
#[specta::specta]
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
#[derive(Serialize, specta::Type)]
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
#[specta::specta]
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
#[specta::specta]
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
        client_token: None,
        yggdrasil_base: None,
    })
}

/// Add (or update) an offline account from a username and select it.
#[tauri::command]
#[specta::specta]
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
        client_token: None,
        yggdrasil_base: None,
    })
}

/// 外置登录(Yggdrasil / authlib-injector):用 base + 用户名 + 密码登录第三方皮肤站,
/// 落库为 Yggdrasil 账号并选中。启动时会自动注入 authlib-injector。
#[tauri::command]
#[specta::specta]
pub async fn yggdrasil_login(
    base: String,
    username: String,
    password: String,
) -> CmdResult<AccountSummary> {
    use mc_core::auth::YggdrasilClient;
    let base = base.trim();
    if base.is_empty() || username.trim().is_empty() {
        return Err("皮肤站地址和用户名不能为空".to_string());
    }
    let client = YggdrasilClient::new(base).with_http(make_downloader()?.client().clone());
    let session = client.authenticate(username.trim(), &password).await.map_err(err)?;
    store_and_select(StoredAccount {
        kind: AccountKind::Yggdrasil,
        username: session.username,
        uuid: session.uuid,
        access_token: session.access_token,
        refresh_token: None,
        xuid: String::new(),
        user_type: "msa".to_string(),
        owns_game: true,
        // 外置登录 token 由皮肤站签发,这里不预判过期;启动前若失效会被皮肤站拒绝。
        expires_at: None,
        client_token: Some(session.client_token),
        yggdrasil_base: Some(client.base().to_string()),
    })
}

/// Switch the active account.
#[tauri::command]
#[specta::specta]
pub fn select_account(uuid: String) -> CmdResult<()> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    store.select(&uuid).map_err(err)?;
    store.save().map_err(err)
}

/// Remove an account by uuid.
#[tauri::command]
#[specta::specta]
pub fn remove_account(uuid: String) -> CmdResult<()> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    store.remove(&uuid);
    store.save().map_err(err)
}

/// 显式刷新当前选中的微软账号的登录(免浏览器,用 refresh_token)。返回是否执行了续期。
/// 失败(refresh_token 失效)时返回错误,UI 据此提示重新登录。
#[tauri::command]
#[specta::specta]
pub async fn refresh_account() -> CmdResult<bool> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    // 显式刷新:用极大 margin 强制对选中的微软账号尝试续期,不看剩余有效期。
    auth::refresh_selected_microsoft(&mut store, &msa_client(), i64::MAX / 2)
        .await
        .map_err(err)
}

// --- skin / cape management (Microsoft accounts only) --------------------

/// 解析指定 uuid 账号的 Minecraft access token。仅微软账号有皮肤 API;
/// 离线 / 外置账号返回清晰错误(占位 token 用不了 profile 端点)。
fn mc_access_token(uuid: &str) -> CmdResult<String> {
    let store = AccountStore::load(accounts_path()).map_err(err)?;
    let acc = store
        .accounts()
        .iter()
        .find(|a| a.uuid == uuid)
        .ok_or_else(|| format!("账号 {uuid} 不存在"))?;
    if acc.kind != AccountKind::Microsoft {
        return Err("只有微软正版账号才能管理皮肤与披风".to_string());
    }
    if acc.access_token.is_empty() || acc.access_token == "0" {
        return Err("该账号缺少有效的登录令牌,请重新登录微软账号".to_string());
    }
    Ok(acc.access_token.clone())
}

/// 读取某微软账号的皮肤 / 披风资料。
#[tauri::command]
#[specta::specta]
pub async fn skin_profile(account_uuid: String) -> CmdResult<mc_core::skin::ProfileSkins> {
    let token = mc_access_token(&account_uuid)?;
    mc_core::skin::fetch_profile(&token).await.map_err(err)
}

/// 上传本地 PNG 作为新皮肤。`variant` 为 `classic` / `slim`。返回更新后的资料。
#[tauri::command]
#[specta::specta]
pub async fn skin_upload(
    account_uuid: String,
    path: String,
    variant: String,
) -> CmdResult<mc_core::skin::ProfileSkins> {
    let token = mc_access_token(&account_uuid)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("读取皮肤文件失败:{e}"))?;
    mc_core::skin::upload_skin(&token, &bytes, &variant).await.map_err(err)
}

/// 设置当前披风(`Some`)或隐藏披风(`None`)。返回更新后的资料。
#[tauri::command]
#[specta::specta]
pub async fn skin_set_cape(
    account_uuid: String,
    cape_id: Option<String>,
) -> CmdResult<mc_core::skin::ProfileSkins> {
    let token = mc_access_token(&account_uuid)?;
    mc_core::skin::set_cape(&token, cape_id.as_deref()).await.map_err(err)
}

#[tauri::command]
#[specta::specta]
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

/// Discover 多选 facet 过滤(可选)。空字段即"不按该维度过滤"。仅 Modrinth 消费这些
/// (Modrinth 把 loader 放在 categories 维度、环境是 `client_side`/`server_side` facet);
/// `provider==curseforge` 时这些被忽略,只用顶层 `game_version` / `loader`。
#[derive(Debug, Default, serde::Deserialize, specta::Type)]
pub struct SearchFacetsArg {
    /// 多选内容分类(每个各成一个 AND 组)。
    #[serde(default)]
    pub categories: Vec<String>,
    /// 多选 loader(合成一个 OR 组)。
    #[serde(default)]
    pub loaders: Vec<String>,
    /// 多选游戏版本(合成一个 OR 组)。
    #[serde(default)]
    pub game_versions: Vec<String>,
    /// 运行环境:`"client"` / `"server"`(其余忽略)。
    #[serde(default)]
    pub environment: Option<String>,
    /// 仅开源项目(License facet)。
    #[serde(default)]
    pub open_source: Option<bool>,
}

/// 跨平台内容搜索:`provider` 缺省 `modrinth`(也可 `curseforge`,需配 CF key),`sort`
/// 缺省按相关度。`facets` 是 Discover 的多选 facet 过滤(仅 Modrinth 消费)。经 Provider
/// 注册表路由,统一返回 [`SearchHit`]。命令名保持 `modrinth_search` 以稳定绑定,但实际是
/// 泛平台搜索。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub async fn modrinth_search(
    query: String,
    kind: String,
    game_version: Option<String>,
    loader: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    provider: Option<String>,
    sort: Option<String>,
    facets: Option<SearchFacetsArg>,
) -> CmdResult<Vec<mc_core::modplatform::SearchHit>> {
    use mc_core::modplatform::{SearchQuery, SortMethod};
    let kind = match kind.as_str() {
        "modpack" => ResourceKind::Modpack,
        "shader" => ResourceKind::Shader,
        "resourcepack" => ResourceKind::ResourcePack,
        "datapack" => ResourceKind::Datapack,
        _ => ResourceKind::Mod,
    };
    let sort = match sort.as_deref() {
        Some("downloads") => SortMethod::Downloads,
        Some("newest") => SortMethod::Newest,
        Some("updated") => SortMethod::Updated,
        _ => SortMethod::Relevance,
    };
    let facets = facets.unwrap_or_default();
    let q = SearchQuery {
        text: query,
        kind,
        game_version: game_version.filter(|s| !s.is_empty()),
        loader: loader.filter(|s| !s.is_empty()),
        game_versions: facets.game_versions,
        loaders: facets.loaders,
        categories: facets.categories,
        environment: facets.environment.filter(|s| !s.is_empty()),
        open_source: facets.open_source,
        offset: offset.unwrap_or(0),
        limit: limit.unwrap_or(30),
        sort,
    };
    let pid = parse_provider(provider.as_deref())?;
    let p = provider_or_err(&make_registry(), pid)?;
    p.search(&q).await.map_err(err)
}

/// Modrinth 的 facet 分类法(内容分类 / loader / 游戏版本),供 Discover 渲染过滤面板。
/// 进程内缓存(见 [`ModrinthApi::content_facets`]),仅 Modrinth 提供;CurseForge 不走此处。
#[tauri::command]
#[specta::specta]
pub async fn content_facets() -> CmdResult<mc_core::modplatform::modrinth::FacetTagsDto> {
    ModrinthApi::new().content_facets().await.map_err(err)
}

// --- theme persistence ----------------------------------------------------

fn theme_path() -> PathBuf {
    data_dir().join("theme.json")
}

#[tauri::command]
#[specta::specta]
pub fn get_theme() -> CmdResult<ThemeConfig> {
    match std::fs::read_to_string(theme_path()) {
        Ok(s) => serde_json::from_str(&s).map_err(err),
        Err(_) => Ok(ThemeConfig::default()),
    }
}

#[tauri::command]
#[specta::specta]
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
#[specta::specta]
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
#[specta::specta]
pub async fn launch_instance(
    app: AppHandle,
    state: State<'_, RunningGames>,
    root: String,
    id: String,
    name: String,
    online: bool,
    server: Option<String>,
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
        // 外置登录账号:启动前校验 token,失效则用 client_token 免密续期并写回
        // (best-effort:校验/续期失败就用现有 token 继续,不阻断启动)。
        let _ = auth::refresh_selected_yggdrasil(&mut store, dl.client().clone()).await;
    }

    // 外置登录账号:启动前下载 authlib-injector,并把 `-javaagent` 注入 JVM 参数,
    // 否则游戏仍走 Mojang 认证、外置皮肤/联机校验都不生效。
    let mut extra_jvm_args: Vec<String> = Vec::new();
    if let Some(yg_base) = AccountStore::load(&accounts_path)
        .ok()
        .and_then(|s| s.selected_account().and_then(|a| a.yggdrasil_base.clone()))
    {
        match auth::yggdrasil::download_authlib_injector(&dl, &data_dir().join("authlib")).await {
            Ok(jar) => extra_jvm_args.push(auth::yggdrasil::javaagent_arg(&jar, &yg_base)),
            Err(e) => return Err(format!("下载 authlib-injector 失败:{e}")),
        }
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
        extra_jvm_args,
        server_override: server,
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
#[specta::specta]
pub fn stop_instance(state: State<'_, RunningGames>, id: String) -> CmdResult<()> {
    if let Some(kill) = state.take(&id) {
        let _ = kill.send(());
    }
    Ok(())
}

/// 当前正在运行的实例 id 列表(供 UI 挂载时同步运行态)。
#[tauri::command]
#[specta::specta]
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

#[derive(Serialize, Clone, specta::Type)]
pub struct GameLog {
    line: String,
    level: &'static str,
}

#[derive(Serialize, Clone, specta::Type)]
pub struct GameStarted {
    id: String,
}

#[derive(Serialize, Clone, specta::Type)]
pub struct GameExit {
    id: String,
    /// 进程退出码(被信号杀死时可能为 `None`)。
    code: Option<i32>,
    success: bool,
    /// 非零退出时的人话崩溃原因(诊断命中才有)。
    reason: Option<String>,
    /// 崩溃诊断给出的可执行建议(可能为空)。
    suggestions: Vec<String>,
}

/// 前端 webview 把启动/错误信息报到这里;经全局 tracing 落进统一日志(`[client]` 前缀)。
#[tauri::command]
#[specta::specta]
pub fn log_boot(msg: String) {
    tracing::info!(target: "client", "{msg}");
}

/// 前端统一日志入口:把 webview 的日志按级别转发到全局日志文件(`[client]` 前缀),
/// 与本地数据层(`[daemon]`)的日志汇到同一处,方便对照排查。
/// level ∈ `error` / `warn` / `info` / `debug`(其它按 info 处理)。
#[tauri::command]
#[specta::specta]
pub fn client_log(level: String, message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "client", "{message}"),
        "warn" => tracing::warn!(target: "client", "{message}"),
        "debug" => tracing::debug!(target: "client", "{message}"),
        _ => tracing::info!(target: "client", "{message}"),
    }
}

/// 返回全局日志目录(`<data_dir>/logs`,必要时创建),前端用 shell 打开它。
#[tauri::command]
#[specta::specta]
pub fn open_logs_dir() -> CmdResult<String> {
    let dir = mc_core::paths::logs_dir(&data_dir());
    paths::ensure_dir(&dir).map_err(err)?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 读取最新日志文件的末尾若干行,供应用内日志查看器。日志按日滚动(文件名形如
/// `mc-launcher.log.<日期>`),取修改时间最新的那个;有界读取(末尾最多 512KiB)避免大日志卡 UI。
#[tauri::command]
#[specta::specta]
pub fn read_log_tail(lines: usize) -> CmdResult<String> {
    use std::io::{Read, Seek, SeekFrom};

    let dir = mc_core::paths::logs_dir(&data_dir());
    let newest = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("mc-launcher.log"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p);
    let Some(path) = newest else {
        return Ok(String::new());
    };

    const MAX_BYTES: u64 = 512 * 1024;
    let mut f = std::fs::File::open(&path).map_err(err)?;
    let len = f.metadata().map_err(err)?.len();
    let start = len.saturating_sub(MAX_BYTES);
    f.seek(SeekFrom::Start(start)).map_err(err)?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(err)?;
    let text = String::from_utf8_lossy(&bytes);
    // 从中途开始读时丢掉可能不完整的首行。
    let text: &str = if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest).unwrap_or("")
    } else {
        &text
    };

    let cap = lines.clamp(1, 5000);
    let mut collected: Vec<&str> = text.lines().rev().take(cap).collect();
    collected.reverse();
    Ok(collected.join("\n"))
}

/// 拉取轻量后端(mc-server)的新闻/公告。后端未运行/不可达时返回错误,UI 降级到空/错误态。
#[tauri::command]
#[specta::specta]
pub async fn fetch_news() -> CmdResult<Vec<mc_core::server::NewsItem>> {
    let client = mc_core::server::ServerClient::new().map_err(err)?;
    client.news().await.map_err(err)
}

// --- kobeMC account (our backend: better-auth email/password) ----------------
// These reuse one shared ServerClient held in Tauri state (lib.rs `.manage`) so
// the better-auth session cookie persists across calls within an app session.

/// Build the shared mc-server client (managed in Tauri state).
pub fn kobe_client() -> mc_core::server::ServerClient {
    mc_core::server::ServerClient::new().expect("build mc-server client")
}

/// Register a kobeMC account (email/password); establishes the session.
#[tauri::command]
#[specta::specta]
pub async fn kobemc_signup(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    email: String,
    password: String,
    name: String,
) -> CmdResult<mc_core::server::AuthUser> {
    client.register(&email, &password, &name).await.map_err(err)
}

/// Log in to a kobeMC account; establishes the session cookie.
#[tauri::command]
#[specta::specta]
pub async fn kobemc_login(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    email: String,
    password: String,
) -> CmdResult<mc_core::server::AuthUser> {
    client.login(&email, &password).await.map_err(err)
}

/// The current kobeMC session user, or `None` if not logged in.
#[tauri::command]
#[specta::specta]
pub async fn kobemc_session(
    client: tauri::State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Option<mc_core::server::AuthUser>> {
    Ok(client.me().await.ok())
}

/// Log out of the kobeMC account (clears the server session).
#[tauri::command]
#[specta::specta]
pub async fn kobemc_logout(
    client: tauri::State<'_, mc_core::server::ServerClient>,
) -> CmdResult<()> {
    client.logout().await.map_err(err)
}

// --- private realms (临时领域) + the syncer ----------------------------------
// Thin glue over mc_core::realm: realm CRUD on the held kobeMC ServerClient, and
// the syncer that reconciles an instance's mods/ to a realm manifest. Building a
// manifest from an instance resolves local mod jars to download urls by hash
// (Modrinth provider); the reconcile downloads missing/changed files and can drop
// mods the manifest no longer carries.

use mc_core::realm::{CreateRealmReq, RealmManifest, RealmMember, RealmSummary, SyncPlan, SyncReport};
use mc_core::types::RealmRef;

/// Resolve an instance from a game root + id.
fn instance_of(root: &str, id: &str) -> Instance {
    Instance::new(id, root_paths(root).root().to_path_buf())
}

/// Build the local realm binding (stored on the instance) from a server summary.
/// `loader_version` is filled later from the manifest on "begin" — the summary
/// doesn't carry it.
fn realm_ref(s: &RealmSummary, role: &str) -> RealmRef {
    RealmRef {
        realm_id: s.id.clone(),
        code: Some(s.code.clone()),
        role: role.to_string(),
        name: Some(s.name.clone()),
        mc_version: s.mc_version.clone(),
        loader: s.loader.clone(),
        loader_version: None,
    }
}

/// Build a full snapshot (manifest + optional overrides zip) from a host's
/// instance via the Modrinth provider (always available — no API key needed).
async fn snapshot_of_instance(
    root: &str,
    id: &str,
    mc_version: &str,
    loader: &str,
    loader_version: Option<String>,
) -> CmdResult<(RealmManifest, Option<Vec<u8>>)> {
    let inst = instance_of(root, id);
    let reg = make_registry();
    let provider = provider_or_err(&reg, mc_core::modplatform::ProviderId::Modrinth)?;
    mc_core::realm::build_snapshot(&inst, provider.as_ref(), mc_version, loader, loader_version)
        .await
        .map_err(err)
}

/// Download + extract the realm's overrides blob into `inst` when the manifest
/// carries one. Best-effort: a missing/failed blob doesn't fail the whole sync.
/// Extraction runs on a blocking thread (blobs can be large).
async fn apply_overrides_if_any(
    client: &mc_core::server::ServerClient,
    realm_id: &str,
    inst: &Instance,
    manifest: &RealmManifest,
) {
    if manifest.overrides.is_none() {
        return;
    }
    if let Ok(zip) = client.download_overrides(realm_id).await {
        let inst = inst.clone();
        let _ = tokio::task::spawn_blocking(move || mc_core::realm::apply_overrides(&inst, &zip)).await;
    }
}

/// Realms the logged-in user belongs to.
#[tauri::command]
#[specta::specta]
pub async fn realm_list(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<RealmSummary>> {
    client.list_realms().await.map_err(err)
}

/// A single realm's summary.
#[tauri::command]
#[specta::specta]
pub async fn realm_get(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
) -> CmdResult<RealmSummary> {
    client.get_realm(&realm_id).await.map_err(err)
}

/// Share an instance as a realm: create it from the instance's current mods, then
/// stamp the realm binding onto that instance (host = owner). Returns the summary.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn realm_create(
    client: State<'_, mc_core::server::ServerClient>,
    root: String,
    instance_id: String,
    name: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
    expires_in_secs: Option<i64>,
) -> CmdResult<RealmSummary> {
    let (manifest, overrides) =
        snapshot_of_instance(&root, &instance_id, &mc_version, &loader, loader_version).await?;
    let summary = client
        .create_realm(&CreateRealmReq { name, expires_in_secs, manifest })
        .await
        .map_err(err)?;
    if let Some(zip) = overrides {
        client.upload_overrides(&summary.id, zip).await.map_err(err)?;
    }
    let paths = root_paths(&root);
    let _ = mc_core::instance::lifecycle::set_instance_realm(
        &paths,
        &instance_id,
        Some(realm_ref(&summary, "owner")),
    );
    Ok(summary)
}

/// Join a realm by code and create a **pending** local instance bound to it (no
/// core installed yet — that's "begin"). Returns the new instance id, or `None`
/// if the code is unknown/expired.
#[tauri::command]
#[specta::specta]
pub async fn realm_join(
    client: State<'_, mc_core::server::ServerClient>,
    root: String,
    code: String,
) -> CmdResult<Option<String>> {
    let Some(summary) = client.join_realm(code.trim()).await.map_err(err)? else {
        return Ok(None);
    };
    let paths = root_paths(&root);
    let g = settings_global();
    let id = mc_core::instance::lifecycle::create_realm_shell(
        &paths,
        &summary.name,
        realm_ref(&summary, &summary.role),
        g.default_memory_mb,
        g.java_path.clone(),
    )
    .map_err(err)?;
    Ok(Some(id))
}

/// "Begin": for a freshly-joined (pending) instance, install the core (version +
/// loader from the manifest) then download the realm's mods. Idempotent on the
/// core. Progress streams over `realm://sync-progress`.
#[tauri::command]
#[specta::specta]
pub async fn realm_begin(
    app: AppHandle,
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<SyncReport> {
    let paths = root_paths(&root);
    let inst = instance_of(&root, &instance_id);
    let manifest = client.realm_manifest(&realm_id).await.map_err(err)?;
    let dl = make_downloader()?;
    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "realm://sync-progress", rx);

    // 1) install the core (version + loader) — idempotent.
    let mc_version = manifest.mc_version.clone().unwrap_or_default();
    let loader_opt = match parse_loader_kind(manifest.loader.as_deref().unwrap_or("")) {
        None | Some(mc_core::types::LoaderKind::Vanilla) => None,
        Some(kind) => Some((kind, manifest.loader_version.clone().unwrap_or_default())),
    };
    mc_core::instance::lifecycle::materialize_core(&dl, &paths, &instance_id, &mc_version, loader_opt, Some(tx.clone()))
        .await
        .map_err(err)?;

    // 2) download the realm's mods.
    let plan = mc_core::realm::plan_sync(&inst, &manifest);
    let report = mc_core::realm::apply_sync(&inst, &dl, &plan, false, Some(tx)).await.map_err(err)?;

    // 3) extract the overrides blob (config/scripts/non-CDN content), if any.
    apply_overrides_if_any(&client, &realm_id, &inst, &manifest).await;

    let _ = client.mark_realm_synced(&realm_id, report.version).await;
    Ok(report)
}

/// Member list (with synced-version progress).
#[tauri::command]
#[specta::specta]
pub async fn realm_members(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
) -> CmdResult<Vec<RealmMember>> {
    client.realm_members(&realm_id).await.map_err(err)
}

/// Owner/admin republishes the manifest from an instance; returns new version.
#[tauri::command]
#[specta::specta]
pub async fn realm_push_manifest(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
) -> CmdResult<i32> {
    let (manifest, overrides) =
        snapshot_of_instance(&root, &instance_id, &mc_version, &loader, loader_version).await?;
    let version = client.push_realm_manifest(&realm_id, &manifest).await.map_err(err)?;
    if let Some(zip) = overrides {
        client.upload_overrides(&realm_id, zip).await.map_err(err)?;
    }
    Ok(version)
}

/// Dry-run: what syncing `instance_id` to the realm's manifest would change.
#[tauri::command]
#[specta::specta]
pub async fn realm_plan_sync(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<SyncPlan> {
    let manifest = client.realm_manifest(&realm_id).await.map_err(err)?;
    Ok(mc_core::realm::plan_sync(&instance_of(&root, &instance_id), &manifest))
}

/// Reconcile `instance_id` to the realm manifest: download missing/changed mods,
/// optionally drop the ones the manifest no longer carries, then report progress
/// to the server. Progress streams over a dedicated `realm://sync-progress` event
/// (kept off `install://progress` so it can't collide with a concurrent install).
#[tauri::command]
#[specta::specta]
pub async fn realm_sync(
    app: AppHandle,
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
    remove_extras: bool,
) -> CmdResult<SyncReport> {
    let inst = instance_of(&root, &instance_id);
    let manifest = client.realm_manifest(&realm_id).await.map_err(err)?;
    let plan = mc_core::realm::plan_sync(&inst, &manifest);

    let dl = make_downloader()?;
    let (tx, rx) = watch::channel(Progress::new("同步领域"));
    forward_progress(app, "realm://sync-progress", rx);
    let report = mc_core::realm::apply_sync(&inst, &dl, &plan, remove_extras, Some(tx)).await.map_err(err)?;

    // Extract the overrides blob (config/scripts/non-CDN content), if any.
    apply_overrides_if_any(&client, &realm_id, &inst, &manifest).await;

    // Best-effort: record how far this member has synced (don't fail the sync).
    let _ = client.mark_realm_synced(&realm_id, report.version).await;
    Ok(report)
}

/// Owner sets a member's role (`admin`/`member`).
#[tauri::command]
#[specta::specta]
pub async fn realm_set_role(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
    role: String,
) -> CmdResult<()> {
    client.set_member_role(&realm_id, &user_id, &role).await.map_err(err)
}

/// Owner removes another member (their own client clears its binding locally).
#[tauri::command]
#[specta::specta]
pub async fn realm_remove_member(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
) -> CmdResult<()> {
    client.remove_member(&realm_id, &user_id).await.map_err(err)
}

/// Owner/admin invites an accepted friend straight into the realm (no join code).
#[tauri::command]
#[specta::specta]
pub async fn realm_invite(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
) -> CmdResult<()> {
    client.realm_invite(&realm_id, &user_id).await.map_err(err)
}

/// Self-leave a realm and unbind it from the local instance (the instance stays;
/// if it was never synced it's just an empty shell that drops out of the list).
#[tauri::command]
#[specta::specta]
pub async fn realm_leave(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<()> {
    client.remove_member(&realm_id, &user_id).await.map_err(err)?;
    let paths = root_paths(&root);
    let _ = mc_core::instance::lifecycle::set_instance_realm(&paths, &instance_id, None);
    Ok(())
}

/// Owner disbands the realm and unbinds it from the local instance.
#[tauri::command]
#[specta::specta]
pub async fn realm_disband(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<()> {
    client.disband_realm(&realm_id).await.map_err(err)?;
    let paths = root_paths(&root);
    let _ = mc_core::instance::lifecycle::set_instance_realm(&paths, &instance_id, None);
    Ok(())
}

// --- friends (username search + request/accept over the held kobeMC client) ---

use mc_core::friend::UserBrief;

/// Set the current user's username (required before friend search works).
#[tauri::command]
#[specta::specta]
pub async fn friend_set_username(
    client: State<'_, mc_core::server::ServerClient>,
    username: String,
) -> CmdResult<()> {
    client.set_username(username.trim()).await.map_err(err)
}

/// Search users by username prefix.
#[tauri::command]
#[specta::specta]
pub async fn friend_search(
    client: State<'_, mc_core::server::ServerClient>,
    q: String,
) -> CmdResult<Vec<UserBrief>> {
    client.search_users(q.trim()).await.map_err(err)
}

/// Send a friend request by user id.
#[tauri::command]
#[specta::specta]
pub async fn friend_request(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_request(&user_id).await.map_err(err)
}

/// Accepted friends.
#[tauri::command]
#[specta::specta]
pub async fn friend_list(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<UserBrief>> {
    client.friends().await.map_err(err)
}

/// Incoming pending friend requests.
#[tauri::command]
#[specta::specta]
pub async fn friend_requests(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<UserBrief>> {
    client.friend_requests().await.map_err(err)
}

/// Accept a pending request from `user_id`.
#[tauri::command]
#[specta::specta]
pub async fn friend_accept(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_accept(&user_id).await.map_err(err)
}

/// Decline a pending request from `user_id`.
#[tauri::command]
#[specta::specta]
pub async fn friend_decline(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_decline(&user_id).await.map_err(err)
}

/// Remove a friend.
#[tauri::command]
#[specta::specta]
pub async fn friend_remove(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_remove(&user_id).await.map_err(err)
}

// --- account linking (bind Microsoft identity to the kobeMC user) ------------

use mc_core::account::Identity;

/// Bind a Microsoft identity to the current kobeMC user. `account_id` is the
/// selected Microsoft account's Minecraft profile UUID; `username` is its
/// gamertag/MC username at link time (informational).
#[tauri::command]
#[specta::specta]
pub async fn account_link_microsoft(
    client: State<'_, mc_core::server::ServerClient>,
    account_id: String,
    username: Option<String>,
) -> CmdResult<()> {
    client.link_microsoft(account_id.trim(), username).await.map_err(err)
}

/// List the current kobeMC user's linked identities.
#[tauri::command]
#[specta::specta]
pub async fn account_identities(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<Identity>> {
    client.account_identities().await.map_err(err)
}

/// Unlink a provider (e.g. `microsoft`) from the current kobeMC user.
#[tauri::command]
#[specta::specta]
pub async fn account_unlink(
    client: State<'_, mc_core::server::ServerClient>,
    provider: String,
) -> CmdResult<()> {
    client.unlink_provider(provider.trim()).await.map_err(err)
}

// --- modpack import / export (thin glue over mc_core::modpack) ---------------

/// 一个 blocked 文件(CurseForge 作者禁第三方分发)的 UI 视图:需用户手动下载。
#[derive(Serialize, specta::Type)]
pub struct BlockedFileDto {
    pub name: String,
    pub website_url: String,
    pub target_dir: String,
    pub required: bool,
}

/// `import_modpack` 的返回:建好的实例 id + 需手动处理的 blocked 文件 + 跳过的可选文件。
#[derive(Serialize, specta::Type)]
pub struct ImportOutcomeDto {
    pub instance_id: String,
    pub blocked: Vec<BlockedFileDto>,
    pub skipped_optional: Vec<String>,
}

/// 导入一个整合包(`.mrpack` / CurseForge zip / MultiMC / MCBBS,自动识别格式),
/// 建好实例并返回其 id。`path` 可为归档文件,**或**未解压的 MultiMC/Prism 实例目录。
/// `blocked` 列出需用户手动下载的 CurseForge 文件。
#[tauri::command]
#[specta::specta]
pub async fn import_modpack(
    app: AppHandle,
    root: String,
    path: String,
    instance_id: Option<String>,
) -> CmdResult<ImportOutcomeDto> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};

    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let engine = ImportEngine::with_defaults(dl, make_registry());

    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = instance_id;

    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "install://progress", rx);
    let outcome = engine
        .import_with_progress(ImportSource::LocalFile(PathBuf::from(path)), opts, Some(tx))
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
#[specta::specta]
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
#[specta::specta]
pub async fn install_modrinth_modpack(
    app: AppHandle,
    root: String,
    project_id: String,
    instance_id: Option<String>,
) -> CmdResult<ImportOutcomeDto> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource, ManagedPack};

    // 1) 取最新版本的 .mrpack 下载地址。
    let api = ModrinthApi::new();
    let versions = api.get_versions(&project_id, None, None).await.map_err(err)?;
    let version = versions
        .into_iter()
        .next()
        .ok_or_else(|| format!("整合包 {project_id} 没有可用版本"))?;
    let version_id = version.id.clone();
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
    let engine = ImportEngine::with_defaults(dl, make_registry());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = instance_id;
    // 记录确切来源(Modrinth 项目 + 安装的版本),持久化到实例 instance.json 的 source。
    opts.managed = Some(ManagedPack {
        platform: "modrinth".to_string(),
        project_id: project_id.clone(),
        version_id: Some(version_id),
    });
    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "install://progress", rx);
    let outcome = engine
        .import_with_progress(ImportSource::Url(url), opts, Some(tx))
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

/// 列出一个项目的所有版本详情(详情页用:版本号/类型/MC/loader/发布时间/下载数/changelog
/// + 该版本下载地址)。`provider` 缺省 `modrinth`。CurseForge 经 provider 的统一版本模型
/// 映射成同一 [`VersionDetail`] 形状(无 changelog/发布时间等富信息时留空),保持绑定稳定。
#[tauri::command]
#[specta::specta]
pub async fn modrinth_versions(
    project_id: String,
    provider: Option<String>,
) -> CmdResult<Vec<mc_core::modplatform::modrinth::VersionDetail>> {
    use mc_core::modplatform::modrinth::VersionDetail;
    match parse_provider(provider.as_deref())? {
        mc_core::modplatform::ProviderId::Modrinth => {
            ModrinthApi::new().version_details(&project_id).await.map_err(err)
        }
        id @ mc_core::modplatform::ProviderId::CurseForge => {
            let p = provider_or_err(&make_registry(), id)?;
            let versions = p.list_versions(&project_id, None, None).await.map_err(err)?;
            Ok(versions
                .into_iter()
                .map(|v| {
                    let file = v.primary_file();
                    let (url, filename, size) = match file {
                        Some(f) if !f.url.is_empty() => {
                            (Some(f.url.clone()), Some(f.filename.clone()), f.size)
                        }
                        _ => (None, None, None),
                    };
                    VersionDetail {
                        id: v.id,
                        version_number: v.version_number,
                        name: v.name,
                        version_type: "release".to_string(),
                        game_versions: v.game_versions,
                        loaders: v.loaders,
                        date_published: String::new(),
                        downloads: 0,
                        changelog: String::new(),
                        mrpack_url: url,
                        mrpack_filename: filename,
                        file_size: size,
                    }
                })
                .collect())
        }
    }
}

/// 检查某实例(由 Modrinth 整合包安装)是否有更新:返回比当前来源版本更新的版本列表。
/// 非整合包来源 / 非 modrinth / 缺 project_id 时返回空(前端据此不显示更新提示)。
#[tauri::command]
#[specta::specta]
pub async fn check_modpack_updates(
    root: String,
    id: String,
) -> CmdResult<Vec<mc_core::modplatform::modrinth::VersionDetail>> {
    let inst = Instance::new(&id, root_paths(&root).root().to_path_buf());
    let Some(src) = inst.load_config().map_err(err)?.source else {
        return Ok(Vec::new());
    };
    if src.provider != "modrinth" {
        return Ok(Vec::new());
    }
    let all = ModrinthApi::new().version_details(&src.project_id).await.map_err(err)?;
    Ok(mc_core::modpack::update::newer_versions(all, src.version_id.as_deref()))
}

/// 整合包就地更新的返回:实例 id + 被清理的旧包文件 + 仍需手动下载 / 跳过的文件。
#[derive(Serialize, specta::Type)]
pub struct ModpackUpdateDto {
    pub instance_id: String,
    /// 因新版本移除而被移入回收站的旧包文件相对路径。
    pub removed: Vec<String>,
    pub blocked: Vec<BlockedFileDto>,
    pub skipped_optional: Vec<String>,
}

/// 把一个由 Modrinth 整合包安装的实例**就地更新**到指定版本:覆盖导入新包到既有实例,
/// 再清理新版移除的受管理文件(移入回收站)。存档 / 实例配置 / 用户自行添加的 mod 均保留。
#[tauri::command]
#[specta::specta]
pub async fn apply_modpack_update(
    app: AppHandle,
    root: String,
    id: String,
    version_id: String,
) -> CmdResult<ModpackUpdateDto> {
    use mc_core::modpack::import::ImportEngine;

    let paths = root_paths(&root);
    let inst = Instance::new(id.as_str(), paths.root().to_path_buf());
    let src = inst
        .load_config()
        .map_err(err)?
        .source
        .ok_or_else(|| "该实例没有整合包来源,无法更新".to_string())?;
    if src.provider != "modrinth" {
        return Err("目前仅支持更新 Modrinth 整合包".to_string());
    }

    // 解析目标版本与旧版本的 .mrpack 下载地址(旧版用于算出被移除的文件)。
    let api = ModrinthApi::new();
    let details = api.version_details(&src.project_id).await.map_err(err)?;
    let new = details
        .iter()
        .find(|v| v.id == version_id)
        .ok_or_else(|| format!("目标版本 {version_id} 不存在"))?;
    let new_url = new
        .mrpack_url
        .clone()
        .ok_or_else(|| "目标版本没有可下载的 .mrpack 文件".to_string())?;
    let old_url = src
        .version_id
        .as_deref()
        .and_then(|vid| details.iter().find(|v| v.id == vid))
        .and_then(|v| v.mrpack_url.clone());

    let engine = ImportEngine::with_defaults(make_downloader()?, make_registry());
    let index_dl = make_downloader()?;
    let (tx, rx) = watch::channel(Progress::new("准备更新"));
    forward_progress(app, "install://progress", rx);
    let outcome = mc_core::modpack::update::apply_modpack_update(
        &engine,
        &index_dl,
        &paths,
        &id,
        &src.project_id,
        &version_id,
        &new_url,
        old_url.as_deref(),
        Some(tx),
    )
    .await
    .map_err(err)?;

    Ok(ModpackUpdateDto {
        instance_id: outcome.instance_id,
        removed: outcome.removed,
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

/// 取一个 Modrinth 项目的完整详情(简介标签页用:长描述正文 markdown + 画廊 +
/// 关注数 + 源码/issue/wiki/discord 等外部链接)。
#[tauri::command]
#[specta::specta]
pub async fn modrinth_project(
    project_id: String,
) -> CmdResult<mc_core::modplatform::modrinth::ProjectDetail> {
    // 走本地持久缓存:实例详情头部 + 概览每次打开都要这份数据,缓存 24h 避免每次都打 Modrinth
    // (抓取失败时回退旧缓存,离线也能显示)。
    let cache = data_dir().join("cache");
    ModrinthApi::new()
        .project_details_cached(&project_id, &cache, std::time::Duration::from_secs(24 * 3600))
        .await
        .map_err(err)
}

/// 从一个 `.mrpack` 直链安装整合包(详情页「安装此版本」用)。
#[tauri::command]
#[specta::specta]
pub async fn install_modpack_url(
    app: AppHandle,
    root: String,
    url: String,
    instance_id: Option<String>,
) -> CmdResult<ImportOutcomeDto> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};

    let paths = root_paths(&root);
    let dl = make_downloader()?;
    let engine = ImportEngine::with_defaults(dl, make_registry());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = instance_id;
    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "install://progress", rx);
    let outcome = engine
        .import_with_progress(ImportSource::Url(url), opts, Some(tx))
        .await
        .map_err(err)?;
    Ok(outcome.into())
}

/// 浏览安装整合包(provider 感知,详情页「安装此版本」用):给定 `(provider, project, version_id)`,
/// 经对应平台解析出整合包归档(Modrinth `.mrpack` / CurseForge `.zip`)的下载直链,再走与
/// [`install_modpack_url`] 完全相同的导入引擎(下载 → 识别格式 → 安装原版+loader+mods+overrides)。
///
/// `provider` 缺省 `modrinth`。`name` 作为目标实例 id(`None` 时由整合包名派生唯一 id)。
/// 安装的版本会写进实例 `instance.json` 的 source,供后续「检查更新」溯源。
///
/// CurseForge 作者禁第三方分发时平台不给整合包直链(`file.url` 为空),此处把该包文件经
/// [`ImportOutcomeDto::blocked`] 的既有机制回传,让前端引导手动下载,而非抛不透明错误。
#[tauri::command]
#[specta::specta]
pub async fn install_modpack(
    app: AppHandle,
    root: String,
    provider: Option<String>,
    project: String,
    version_id: String,
    name: Option<String>,
    icon_url: Option<String>,
) -> CmdResult<ImportOutcomeDto> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource, ManagedPack};

    let id = parse_provider(provider.as_deref())?;

    // 解析整合包归档的下载直链 + 记录溯源平台名。
    let (url, platform) = match id {
        mc_core::modplatform::ProviderId::Modrinth => {
            // Modrinth:逐版本拉 .mrpack(主文件即整合包)。
            let api = ModrinthApi::new();
            let versions = api.get_versions(&project, None, None).await.map_err(err)?;
            let version = versions
                .into_iter()
                .find(|v| v.id == version_id)
                .ok_or_else(|| format!("整合包版本 {version_id} 不存在"))?;
            let url = version
                .files
                .iter()
                .find(|f| f.filename.ends_with(".mrpack"))
                .or_else(|| version.primary_file())
                .ok_or_else(|| "该整合包版本没有可下载的 .mrpack 文件".to_string())?
                .url
                .clone();
            (url, "modrinth")
        }
        id @ mc_core::modplatform::ProviderId::CurseForge => {
            // CurseForge:provider 把 (project, fileId) 批量解析成文件;整合包 .zip 即该文件。
            let p = provider_or_err(&make_registry(), id)?;
            let resolved = p
                .get_files_bulk(&[(project.clone(), version_id.clone())])
                .await
                .map_err(err)?
                .into_iter()
                .next()
                .ok_or_else(|| format!("整合包版本 {version_id} 不存在"))?;
            // 作者禁分发 → url 为空:不报错,经 blocked 机制把该整合包文件回传给前端引导手动下载。
            if resolved.file.url.trim().is_empty() {
                return Ok(ImportOutcomeDto {
                    instance_id: String::new(),
                    blocked: vec![cf_blocked_dto(&project, &version_id, &resolved.file.filename, ".")],
                    skipped_optional: Vec::new(),
                });
            }
            (resolved.file.url, "curseforge")
        }
    };

    // 与 install_modpack_url 同路径:引擎先下到临时文件,再识别格式 + 安装。
    let paths = root_paths(&root);
    let dl = make_downloader()?;
    // 实例图标:把整合包项目图标下到临时文件,作为 ImportOptions.icon 拷进实例,使其保留原 logo
    // 而非默认像素占位(失败不致命 → 退回默认)。在 dl 移入引擎前用引用下载。
    let icon_path = match icon_url.filter(|u| !u.trim().is_empty()) {
        Some(u) => match dl.get_bytes(&u).await {
            Ok(bytes) => {
                let safe: String = project.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
                let tmp = std::env::temp_dir().join(format!("mc-modpack-icon-{}-{}.img", std::process::id(), safe));
                std::fs::write(&tmp, &bytes).ok().map(|_| tmp)
            }
            Err(_) => None,
        },
        None => None,
    };
    let engine = ImportEngine::with_defaults(dl, make_registry());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = name;
    opts.icon = icon_path;
    opts.managed = Some(ManagedPack {
        platform: platform.to_string(),
        project_id: project,
        version_id: Some(version_id),
    });
    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app, "install://progress", rx);
    let outcome = engine
        .import_with_progress(ImportSource::Url(url), opts, Some(tx))
        .await
        .map_err(err)?;
    Ok(outcome.into())
}

/// 读取全局设置(下载源/并发/默认内存/Java 路径/语言…)。缺失/损坏回退默认。
#[tauri::command]
#[specta::specta]
pub fn get_settings() -> CmdResult<mc_core::settings::GlobalSettings> {
    mc_core::settings::GlobalSettings::load(&data_dir()).map_err(err)
}

/// 持久化全局设置(原子写 settings.json)。下载相关项下次构造下载器即生效。
#[tauri::command]
#[specta::specta]
pub fn set_settings(settings: mc_core::settings::GlobalSettings) -> CmdResult<()> {
    settings.save(&data_dir()).map_err(err)
}

/// 当前生效的「显示社交 UI」(kobeMC 账号 / 领域 / 好友)开关:用户显式设置优先,
/// 否则按部署场景默认(便携·和实例同级 → 关;桌面独立版 → 开)。
#[tauri::command]
#[specta::specta]
pub fn social_enabled() -> CmdResult<bool> {
    Ok(settings_global()
        .social_enabled
        .unwrap_or_else(|| !mc_core::paths::is_portable_deployment(&exe_dir())))
}
