//! `mc` — a headless CLI front-end over `mc-core`. It exists to drive and verify
//! the engine without any UI (milestone M3 in `docs/04`): list versions, install
//! a version, log in, and launch the game.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use mc_core::auth::{
    self, offline_session, AccountStore, MsaClient, StoredAccount, YggdrasilClient,
};
use mc_core::download::{Downloader, MirrorResolver};
use mc_core::instance::Instance;
use mc_core::launch::{self, LaunchSpec};
use mc_core::paths::{self, GamePaths};
use mc_core::settings::GlobalSettings;
use mc_core::types::{AccountKind, ReleaseKind};
use mc_core::{java, meta, LAUNCHER_NAME, LAUNCHER_VERSION};


mod content;
mod game;
mod session;
use content::*;
use game::*;
use session::*;

#[derive(Parser)]
#[command(name = "mc", version, about = "A fast Minecraft launcher core (CLI)")]
struct Cli {
    /// Game root directory override. Defaults to the first discovered root.
    #[arg(long, global = true)]
    dir: Option<PathBuf>,

    /// Route downloads through the BMCLAPI mirror (faster in China).
    #[arg(long, global = true)]
    mirror: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover game-root directories (portable / official / custom).
    Roots,
    /// Detect installed Java runtimes.
    Java,
    /// List installable Minecraft versions from Mojang's manifest.
    Versions {
        /// Include snapshots.
        #[arg(long)]
        snapshot: bool,
        /// Maximum number of versions to print.
        #[arg(long, default_value_t = 25)]
        limit: usize,
    },
    /// List installed instances in the selected root.
    List,
    /// Download and install a Minecraft version.
    Install {
        /// Version id, e.g. "1.20.1".
        id: String,
    },
    /// Create a new instance from scratch (vanilla, optionally + a mod loader).
    Create {
        /// Display name for the instance.
        name: String,
        /// Minecraft version, e.g. "1.20.1".
        mc_version: String,
        /// Loader: vanilla | fabric | quilt | forge | neoforge.
        #[arg(long, default_value = "vanilla")]
        loader: String,
        /// Loader version (forge build / neoforge version; required for those two).
        #[arg(long)]
        loader_version: Option<String>,
    },
    /// Install Fabric loader for a Minecraft version (installs vanilla if needed).
    Fabric {
        /// Minecraft version, e.g. "1.20.1".
        mc_version: String,
    },
    /// Install Quilt loader for a Minecraft version.
    Quilt { mc_version: String },
    /// Install Forge for a Minecraft version (runs the official installer).
    Forge {
        mc_version: String,
        /// Forge build number, e.g. "47.4.10".
        forge_build: String,
    },
    /// Install NeoForge by version, e.g. "20.4.237" (MC version derived).
    Neoforge { neo_version: String },
    /// Download an Adoptium JRE of the given major version into the data dir.
    JavaInstall { major: u8 },
    /// Analyze a game log file and explain the likely crash cause.
    Crash {
        /// Path to a log / crash report file.
        path: PathBuf,
    },
    /// List mods in an instance.
    Mods {
        /// Instance (version) id.
        id: String,
    },
    /// List worlds/saves in an instance.
    Worlds { id: String },
    /// Install a mod from Modrinth into an instance (resolves required deps).
    InstallMod {
        /// Instance (version) id.
        id: String,
        /// Modrinth project id or slug.
        project: String,
        /// Loader to match (fabric/quilt/forge/neoforge).
        #[arg(long, default_value = "fabric")]
        loader: String,
        /// Minecraft version to match (defaults to the instance id).
        #[arg(long)]
        mc_version: Option<String>,
    },
    /// Query the lite server's aggregated loader versions for a Minecraft version.
    Loaders { mc_version: String },
    /// Check the lite server health (uses MC_SERVER_URL if set).
    ServerHealth,
    /// Register a launcher account on the lite server (better-auth), verify via session.
    RegisterAccount { email: String, password: String },
    /// Launch a version. Uses an offline account unless --account is given.
    Launch {
        /// Version id to launch.
        id: String,
        /// Offline player name (ignored when a stored account is selected).
        #[arg(long, default_value = "Player")]
        name: String,
        /// Use the currently selected stored (Microsoft) account.
        #[arg(long)]
        account: bool,
        /// Verify & download missing files before launching.
        #[arg(long)]
        online: bool,
        /// Explicit Java executable path.
        #[arg(long)]
        java: Option<PathBuf>,
    },
    /// Microsoft device-code login; stores the resulting account.
    Login,
    /// Add (or update) an offline account by username, and select it.
    LoginOffline {
        /// Offline player name.
        name: String,
    },
    /// External (Yggdrasil / authlib-injector) login against a skin site.
    LoginYggdrasil {
        /// authlib-injector API root, e.g. https://littleskin.cn/api/yggdrasil.
        base: String,
        /// Skin-site account (email or username).
        username: String,
        /// Skin-site password.
        password: String,
    },
    /// List stored accounts.
    Accounts,
    /// Search a content platform (Modrinth; --cf for CurseForge, needs MC_CF_API_KEY).
    Search {
        query: String,
        /// Use CurseForge instead of Modrinth.
        #[arg(long)]
        cf: bool,
        /// Resource kind: mod | modpack | shader | resourcepack | datapack.
        #[arg(long, default_value = "mod")]
        kind: String,
        #[arg(long)]
        mc_version: Option<String>,
        #[arg(long)]
        loader: Option<String>,
    },
    /// Reverse-lookup a file by sha512 via Modrinth (the import/export linchpin).
    ResolveHash { sha512: String },
    /// Show a Modrinth project's detail (description body / gallery / links) — the
    /// data behind the detail page's 「简介」 tab.
    Project {
        /// Modrinth project id or slug.
        id: String,
    },
    /// Detect a modpack archive's format without installing anything.
    ModpackDetect { file: PathBuf },
    /// Import a modpack (.mrpack/.zip, auto-detected) — installs vanilla MC + files.
    ModpackImport {
        file: PathBuf,
        #[arg(long)]
        id: Option<String>,
    },
    /// Show or modify the global launcher settings (the same settings.json the UI edits).
    Settings {
        #[command(subcommand)]
        action: Option<SettingsAction>,
    },
    /// Export an instance. target: modrinth | curseforge | modlist[:md|json|csv|txt|html].
    ModpackExport {
        id: String,
        #[arg(default_value = "modrinth")]
        target: String,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        mc_version: Option<String>,
        #[arg(long)]
        loader: Option<String>,
        #[arg(long)]
        loader_version: Option<String>,
    },
}

#[derive(Subcommand)]
enum SettingsAction {
    /// Print the current settings (this is also the default with no sub-action).
    Show,
    /// Modify one or more fields and persist to settings.json.
    Set {
        /// Download source: "official" (direct) or "bmclapi" (China mirror).
        #[arg(long)]
        download_source: Option<String>,
        /// Download concurrency (number of files in flight).
        #[arg(long)]
        concurrency: Option<usize>,
        /// Default heap memory for new instances (MiB).
        #[arg(long)]
        memory_mb: Option<u32>,
        /// Global Java executable path; pass "" to clear (→ auto-detect).
        #[arg(long)]
        java_path: Option<String>,
        /// Force downloads through the mirror even when source is "official".
        #[arg(long)]
        use_mirror: Option<bool>,
        /// UI language tag (e.g. zh-CN / en-US).
        #[arg(long)]
        language: Option<String>,
    },
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn data_dir() -> PathBuf {
    paths::resolve_data_dir(&exe_dir())
}

/// 用户在设置里添加的自定义游戏根目录(让 `custom_roots` 设置参与发现)。
fn custom_roots() -> Vec<PathBuf> {
    GlobalSettings::load(&data_dir())
        .unwrap_or_default()
        .custom_roots
        .iter()
        .map(PathBuf::from)
        .collect()
}

/// Resolve the game root to operate on.
fn resolve_root(dir: &Option<PathBuf>) -> GamePaths {
    if let Some(d) = dir {
        return GamePaths::new(d.clone());
    }
    let roots = paths::discover_roots(&exe_dir(), &data_dir(), &custom_roots());
    let path = roots
        .first()
        .map(|r| PathBuf::from(&r.path))
        .unwrap_or_else(|| data_dir().join(".minecraft"));
    GamePaths::new(path)
}

/// Build a downloader from the persisted [`GlobalSettings`] — the SAME daemon
/// state the desktop UI's settings page reads/writes — so CLI and UI download
/// behavior stay identical (concurrency + mirror source + CurseForge `x-api-key`).
/// `GlobalSettings::downloader` is the one owner of that construction; the global
/// `--mirror` flag then force-enables the full China mirror set on top.
fn downloader(mirror: bool) -> Result<Downloader> {
    let settings = GlobalSettings::load(&data_dir()).unwrap_or_default();
    let dl = settings.downloader().context("building downloader")?;
    Ok(if mirror { dl.with_mirror(MirrorResolver::china()) } else { dl })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let cli = Cli::parse();
    match &cli.command {
        Command::Roots => cmd_roots(),
        Command::Java => cmd_java().await,
        Command::Versions { snapshot, limit } => cmd_versions(&cli, *snapshot, *limit).await,
        Command::List => cmd_list(&cli),
        Command::Install { id } => cmd_install(&cli, id).await,
        Command::Create {
            name,
            mc_version,
            loader,
            loader_version,
        } => cmd_create(&cli, name, mc_version, loader, loader_version.clone()).await,
        Command::Fabric { mc_version } => cmd_fabric(&cli, mc_version).await,
        Command::Quilt { mc_version } => cmd_quilt(&cli, mc_version).await,
        Command::Forge {
            mc_version,
            forge_build,
        } => cmd_forge(&cli, mc_version, forge_build).await,
        Command::Neoforge { neo_version } => cmd_neoforge(&cli, neo_version).await,
        Command::JavaInstall { major } => cmd_java_install(*major).await,
        Command::Crash { path } => cmd_crash(path),
        Command::Mods { id } => cmd_mods(&cli, id),
        Command::Worlds { id } => cmd_worlds(&cli, id),
        Command::InstallMod {
            id,
            project,
            loader,
            mc_version,
        } => cmd_install_mod(&cli, id, project, loader, mc_version.clone()).await,
        Command::Loaders { mc_version } => cmd_loaders(mc_version).await,
        Command::ServerHealth => cmd_server_health().await,
        Command::RegisterAccount { email, password } => cmd_register_account(email, password).await,
        Command::Launch {
            id,
            name,
            account,
            online,
            java,
        } => cmd_launch(&cli, id, name, *account, *online, java.clone()).await,
        Command::Login => cmd_login().await,
        Command::LoginOffline { name } => cmd_login_offline(name),
        Command::LoginYggdrasil { base, username, password } => {
            cmd_login_yggdrasil(base, username, password).await
        }
        Command::Accounts => cmd_accounts(),
        Command::Search {
            query,
            cf,
            kind,
            mc_version,
            loader,
        } => cmd_search(query, *cf, kind, mc_version.clone(), loader.clone()).await,
        Command::ResolveHash { sha512 } => cmd_resolve_hash(sha512).await,
        Command::Project { id } => cmd_project(id).await,
        Command::Settings { action } => cmd_settings(action),
        Command::ModpackDetect { file } => cmd_modpack_detect(file),
        Command::ModpackImport { file, id } => cmd_modpack_import(&cli, file, id.clone()).await,
        Command::ModpackExport {
            id,
            target,
            out,
            name,
            mc_version,
            loader,
            loader_version,
        } => {
            cmd_modpack_export(
                &cli,
                id,
                target,
                out.clone(),
                name.clone(),
                mc_version.clone(),
                loader.clone(),
                loader_version.clone(),
            )
            .await
        }
    }
}
