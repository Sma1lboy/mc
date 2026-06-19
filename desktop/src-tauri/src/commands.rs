//! Tauri commands — a thin glue layer over `mc-core`. Every command maps a UI
//! request to a core call and serialises the result; long operations stream
//! progress / logs back as Tauri events. No launcher logic lives here.

use std::path::{Path, PathBuf};

use mc_core::auth::AccountStore;
use mc_core::download::Downloader;
use mc_core::instance::Instance;
use mc_core::launch::{self, LaunchSpec};
use mc_core::modplatform::modrinth::ModrinthApi;
use mc_core::modplatform::ResourceKind;
use mc_core::types::{
    AccountSummary, GameRoot, InstanceSummary, ManifestVersion, Progress, ThemeConfig,
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
