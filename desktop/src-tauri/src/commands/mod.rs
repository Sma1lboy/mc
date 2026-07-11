//! Tauri commands — a thin glue layer over `mc-core`. Every command maps a UI
//! request to a core call and serialises the result; long operations stream
//! progress / logs back as Tauri events. No launcher logic lives here.
//!
//! One submodule per domain; everything is re-exported flat (glob, so the
//! macro items `#[tauri::command]` generates travel too) — `lib.rs` keeps
//! addressing `commands::foo`. Shared path/provider helpers live below and
//! reach the submodules via their `use super::*;`.

mod agent;
mod account;
mod content;
mod instance;
mod kobe;
mod game;
mod lobby;
mod modpack;
mod modpack_update;
mod realm;
mod social;
mod system;

pub use agent::*;
pub use account::*;
pub use content::*;
pub use instance::*;
pub use kobe::*;
pub use game::*;
pub use lobby::*;
pub use modpack::*;
pub use modpack_update::*;
pub use realm::*;
pub use social::*;
pub use system::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mc_core::agent::tools::{
    apply_diagnostic_operations, cleanup_diagnostic_session, clone_diagnostic_snapshot,
    create_diagnostic_snapshot, refresh_wiki_corpus_cache, tool_build_modpack, tool_diagnose_instance,
    tool_inspect_base_modpack, tool_install_modpack, tool_list_instances, tool_mod_get_detail,
    tool_resolve_mods, tool_search_base_modpacks, tool_search_mods, tool_validate_modpack_plan,
    tool_wiki_open, tool_wiki_search, BuildModpackArgs, BuildModpackOutput, DiagnoseInstanceArgs,
    DiagnoseInstanceOutput, InspectBaseModpackArgs, InspectBaseModpackOutput, InstallModpackArgs,
    InstallModpackOutput, ListInstancesOutput, ModGetDetailArgs, ModGetDetailOutput,
    DiagnosticSandboxSnapshot, DiagnosticTrialOperation, ResolveModsArgs, ResolveModsOutput, SearchBaseModpacksArgs, SearchBaseModpacksOutput,
    SearchModsArgs, SearchModsOutput, ValidateModpackPlanArgs, ValidateModpackPlanOutput,
    WikiOpenArgs, WikiOpenOutput, WikiSearchArgs, WikiSearchOutput,
};
use mc_core::agent::ChatToolsCtx;
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
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{oneshot, watch};

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

use mc_core::paths::{exe_dir, local_data_dir as data_dir};

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

/// 把字符串解析成 loader 家族(导出时把 loader 依赖写进索引)。
/// 走权威逆函数 [`LoaderKind::from_family`],与其余解析点同一份真相。
fn parse_loader_kind(s: &str) -> Option<mc_core::types::LoaderKind> {
    mc_core::types::LoaderKind::from_family(s)
}

/// 账号库路径(单一 owner:`paths::accounts_path`)。
fn accounts_path() -> PathBuf {
    paths::accounts_path(&data_dir())
}

/// Resolve an instance from a game root + id.
fn instance_of(root: &str, id: &str) -> Instance {
    Instance::new(id, root_paths(root).root().to_path_buf())
}

/// 加载全局设置(损坏/缺失回退默认,绝不报错)。
fn settings_global() -> mc_core::settings::GlobalSettings {
    mc_core::settings::GlobalSettings::load(&data_dir()).unwrap_or_default()
}

/// 用户在设置里添加的自定义游戏根目录(让 `custom_roots` 设置真正参与发现)。
fn custom_roots() -> Vec<PathBuf> {
    settings_global().custom_root_paths()
}

/// 按用户设置/环境构造 Provider 注册表:总有 Modrinth;解析出 CurseForge key 才注册 CurseForge。
/// 搜索 / 浏览安装 / 整合包导入导出共用同一份(让「设置里的 CF key」与环境 key 都生效)。
fn make_registry() -> mc_core::modplatform::provider::ProviderRegistry {
    settings_global().provider_registry()
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
/// 实际构造逻辑由 [`GlobalSettings::downloader`] 单一 owner 持有,与 CLI 共用。
fn make_downloader() -> CmdResult<Downloader> {
    settings_global().downloader().map_err(err)
}

/// Refresh the local wiki corpus cache for one installed instance (used by modpack
/// install/import to warm the agent's wiki tools). Falls back to the instance id
/// when the config carries no modpack source project id.
async fn refresh_wiki_cache_for_instance(paths: &paths::GamePaths, id: &str) -> CmdResult<()> {
    let id = id.trim();
    if id.is_empty() {
        return Ok(());
    }
    let inst = Instance::new(id, paths.root().to_path_buf());
    let modpack_id = inst
        .load_config()
        .ok()
        .and_then(|cfg| cfg.source.map(|src| src.project_id))
        .filter(|project_id| !project_id.trim().is_empty())
        .unwrap_or_else(|| id.to_string());
    refresh_wiki_corpus_cache(modpack_id, Some(id.to_string()), &inst.game_dir())
        .await
        .map_err(err)
}

/// Best-effort wiki cache warm after an install/import: log and swallow errors so a
/// cache miss never fails the install itself.
async fn best_effort_refresh_wiki_cache(paths: &paths::GamePaths, id: &str) {
    if let Err(e) = refresh_wiki_cache_for_instance(paths, id).await {
        tracing::warn!(instance_id = %id, error = %e, "failed to rebuild wiki corpus cache");
    }
}

/// Ensure every agent wiki `source_paths` entry resolves inside the active game root
/// (the agent wiki tools read arbitrary paths; this is the trust-boundary check).
fn validate_agent_wiki_source_paths(root: &str, source_paths: &[String]) -> CmdResult<()> {
    if source_paths.is_empty() {
        return Err("wiki source paths are required".into());
    }
    let game_root = root_paths(root).root().canonicalize().map_err(err)?;
    for raw in source_paths {
        let path = PathBuf::from(raw);
        let canonical = path.canonicalize().map_err(err)?;
        if !canonical.starts_with(&game_root) {
            return Err(format!(
                "wiki source path must be inside the active game root: {}",
                path.display()
            ));
        }
    }
    Ok(())
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

/// Forward a [`Progress`] watch channel to a Tauri event until it closes.
fn forward_progress(app: AppHandle, event: &'static str, mut rx: watch::Receiver<Progress>) {
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow().clone();
            let _ = app.emit(event, p);
        }
    });
}

/// Open a progress channel wired to a Tauri `event` and return just the sender to
/// hand to an mc-core operation. One owner for the `watch::channel(...)` +
/// `forward_progress(...)` pair every long-running command used to spell out.
fn progress_channel(app: AppHandle, event: &'static str, initial: &str) -> watch::Sender<Progress> {
    let (tx, rx) = watch::channel(Progress::new(initial));
    forward_progress(app, event, rx);
    tx
}
