//! Tauri commands — a thin glue layer over `mc-core`. Every command maps a UI
//! request to a core call and serialises the result; long operations stream
//! progress / logs back as Tauri events. No launcher logic lives here.

use std::path::{Path, PathBuf};

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
use tauri::{AppHandle, Emitter};
use tokio::sync::watch;

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
    let roots = paths::discover_roots(&exe_dir(), &data_dir(), &[]);
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
    Ok(paths::discover_roots(&exe_dir(), &data_dir(), &[]))
}

#[tauri::command]
pub fn list_instances(root: String) -> CmdResult<Vec<InstanceSummary>> {
    Ok(mc_core::instance::list_instances(&root_paths(&root)))
}

#[tauri::command]
pub async fn list_versions(snapshot: bool) -> CmdResult<Vec<ManifestVersion>> {
    let dl = Downloader::new(32).map_err(err)?;
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
) -> CmdResult<Vec<mc_core::modplatform::SearchHit>> {
    let api = ModrinthApi::new();
    let rk = match kind.as_str() {
        "modpack" => ResourceKind::Modpack,
        "shader" => ResourceKind::Shader,
        "resourcepack" => ResourceKind::ResourcePack,
        "datapack" => ResourceKind::Datapack,
        _ => ResourceKind::Mod,
    };
    api.search(&query, rk, game_version.as_deref(), loader.as_deref(), 30)
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
    let dl = Downloader::new(64).map_err(err)?;
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

#[tauri::command]
pub async fn launch_instance(
    app: AppHandle,
    root: String,
    id: String,
    name: String,
    online: bool,
) -> CmdResult<()> {
    let paths = root_paths(&root);
    let dl = Downloader::new(64).map_err(err)?;

    // Prefer the selected stored account; fall back to an offline session.
    let session = AccountStore::load(data_dir().join("accounts.json"))
        .ok()
        .and_then(|s| s.selected_session())
        .unwrap_or_else(|| auth::offline_session(&name));

    let spec = LaunchSpec {
        instance: Instance::new(&id, paths.root().to_path_buf()),
        session,
        java_path: None,
        launcher_name: LAUNCHER_NAME.to_string(),
        launcher_version: LAUNCHER_VERSION.to_string(),
        online,
    };

    let (tx, rx) = watch::channel(Progress::new("准备"));
    forward_progress(app.clone(), "launch://progress", rx);

    let mut child = launch::launch(spec, &dl, Some(tx)).await.map_err(err)?;

    // Stream the game's stdout/stderr as log events (also drains the pipes so the
    // child never blocks on a full buffer).
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        let app = app.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit("game://log", GameLog { line, level: "info" });
            }
        });
    }
    if let Some(e) = child.stderr.take() {
        let app = app.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(e).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit("game://log", GameLog { line, level: "error" });
            }
        });
    }
    // Reap the process in the background so it doesn't become a zombie.
    tokio::spawn(async move {
        let _ = child.wait().await;
    });
    Ok(())
}

#[derive(Serialize, Clone)]
struct GameLog {
    line: String,
    level: &'static str,
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
    use mc_core::download::MirrorResolver;
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};
    use mc_core::modplatform::provider::ProviderRegistry;

    let paths = root_paths(&root);
    let dl = Downloader::new(16).map_err(err)?.with_mirror(MirrorResolver::china());
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
            input.loader = Some((lk, v));
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
    use mc_core::download::MirrorResolver;
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
    let dl = Downloader::new(16).map_err(err)?.with_mirror(MirrorResolver::china());
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
