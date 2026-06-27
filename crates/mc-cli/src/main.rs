//! `mc` — a headless CLI front-end over `mc-core`. It exists to drive and verify
//! the engine without any UI (milestone M3 in `docs/04`): list versions, install
//! a version, log in, and launch the game.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use mc_core::auth::{
    self, offline_session, now_unix, AccountStore, MsaClient, StoredAccount, YggdrasilClient,
    MC_TOKEN_TTL_SECS,
};
use mc_core::download::{Downloader, MirrorResolver};
use mc_core::instance::Instance;
use mc_core::launch::{self, LaunchSpec};
use mc_core::paths::{self, GamePaths};
use mc_core::settings::GlobalSettings;
use mc_core::{java, meta, LAUNCHER_NAME, LAUNCHER_VERSION};
use mc_core::types::{AccountKind, ReleaseKind};

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
    Quilt {
        mc_version: String,
    },
    /// Install Forge for a Minecraft version (runs the official installer).
    Forge {
        mc_version: String,
        /// Forge build number, e.g. "47.4.10".
        forge_build: String,
    },
    /// Install NeoForge by version, e.g. "20.4.237" (MC version derived).
    Neoforge {
        neo_version: String,
    },
    /// Download an Adoptium JRE of the given major version into the data dir.
    JavaInstall {
        major: u8,
    },
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
    Worlds {
        id: String,
    },
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
    Loaders {
        mc_version: String,
    },
    /// Check the lite server health (uses MC_SERVER_URL if set).
    ServerHealth,
    /// Register a launcher account on the lite server (better-auth), verify via session.
    RegisterAccount {
        email: String,
        password: String,
    },
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
    ResolveHash {
        sha512: String,
    },
    /// Show a Modrinth project's detail (description body / gallery / links) — the
    /// data behind the detail page's 「简介」 tab.
    Project {
        /// Modrinth project id or slug.
        id: String,
    },
    /// Detect a modpack archive's format without installing anything.
    ModpackDetect {
        file: PathBuf,
    },
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
/// behavior stay identical (concurrency + mirror source). The global `--mirror`
/// flag still force-enables the full China mirror set regardless of settings.
fn downloader(mirror: bool) -> Result<Downloader> {
    let settings = GlobalSettings::load(&data_dir()).unwrap_or_default();
    let dl = Downloader::new(settings.concurrency.max(1)).context("building downloader")?;
    let resolver = if mirror { MirrorResolver::china() } else { settings.mirror_resolver() };
    Ok(dl.with_mirror(resolver))
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
        Command::Create { name, mc_version, loader, loader_version } => {
            cmd_create(&cli, name, mc_version, loader, loader_version.clone()).await
        }
        Command::Fabric { mc_version } => cmd_fabric(&cli, mc_version).await,
        Command::Quilt { mc_version } => cmd_quilt(&cli, mc_version).await,
        Command::Forge { mc_version, forge_build } => cmd_forge(&cli, mc_version, forge_build).await,
        Command::Neoforge { neo_version } => cmd_neoforge(&cli, neo_version).await,
        Command::JavaInstall { major } => cmd_java_install(*major).await,
        Command::Crash { path } => cmd_crash(path),
        Command::Mods { id } => cmd_mods(&cli, id),
        Command::Worlds { id } => cmd_worlds(&cli, id),
        Command::InstallMod { id, project, loader, mc_version } => {
            cmd_install_mod(&cli, id, project, loader, mc_version.clone()).await
        }
        Command::Loaders { mc_version } => cmd_loaders(mc_version).await,
        Command::ServerHealth => cmd_server_health().await,
        Command::RegisterAccount { email, password } => {
            cmd_register_account(email, password).await
        }
        Command::Launch { id, name, account, online, java } => {
            cmd_launch(&cli, id, name, *account, *online, java.clone()).await
        }
        Command::Login => cmd_login().await,
        Command::LoginOffline { name } => cmd_login_offline(name),
        Command::LoginYggdrasil { base, username, password } => {
            cmd_login_yggdrasil(base, username, password).await
        }
        Command::Accounts => cmd_accounts(),
        Command::Search { query, cf, kind, mc_version, loader } => {
            cmd_search(query, *cf, kind, mc_version.clone(), loader.clone()).await
        }
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

fn cmd_roots() -> Result<()> {
    let roots = paths::discover_roots(&exe_dir(), &data_dir(), &custom_roots());
    if roots.is_empty() {
        println!("(没有发现游戏根目录)");
    }
    for r in roots {
        println!("[{:?}] {}  ->  {}", r.kind, r.name, r.path);
    }
    Ok(())
}

async fn cmd_java() -> Result<()> {
    let installs = java::detect_all().await;
    if installs.is_empty() {
        println!("未检测到 Java。");
        return Ok(());
    }
    for j in installs {
        println!(
            "Java {}  ({}, {})  {}",
            j.version,
            if j.is_64bit { "64-bit" } else { "32-bit" },
            j.source,
            j.path.display()
        );
    }
    Ok(())
}

async fn cmd_versions(cli: &Cli, snapshot: bool, limit: usize) -> Result<()> {
    let dl = downloader(cli.mirror)?;
    let versions = meta::fetch_manifest(&dl).await.context("fetching version manifest")?;
    let mut shown = 0;
    for v in &versions {
        if !snapshot && v.kind != ReleaseKind::Release {
            continue;
        }
        println!("{:<16} {:?}", v.id, v.kind);
        shown += 1;
        if shown >= limit {
            break;
        }
    }
    println!("\n({} 个版本可安装)", versions.len());
    Ok(())
}

fn cmd_list(cli: &Cli) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let instances = mc_core::instance::list_instances(&paths);
    if instances.is_empty() {
        println!("根目录 {} 下没有已安装实例。", paths.root().display());
        return Ok(());
    }
    for i in instances {
        println!("{:<20} {} [{}]", i.id, i.mc_version, i.loader.as_str());
    }
    Ok(())
}

async fn cmd_install(cli: &Cli, id: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    println!("正在拉取版本清单…");
    let manifest = meta::fetch_manifest(&dl).await?;
    let entry = manifest
        .iter()
        .find(|v| v.id == id)
        .with_context(|| format!("版本 {id} 不在清单中"))?;

    println!("安装 {} 到 {} …", id, paths.root().display());
    let (tx, mut rx) = tokio::sync::watch::channel(mc_core::types::Progress::new("准备"));
    let printer = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow().clone();
            if p.total > 0 {
                println!("  {} {}/{}", p.stage, p.current, p.total);
            }
        }
    });
    launch::install_version(&dl, &paths, entry, Some(tx)).await?;
    let _ = printer.await;
    println!("✓ 安装完成:{id}");
    Ok(())
}

async fn cmd_create(
    cli: &Cli,
    name: &str,
    mc_version: &str,
    loader: &str,
    loader_version: Option<String>,
) -> Result<()> {
    use mc_core::types::LoaderKind;
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;

    let loader_opt = match loader.to_ascii_lowercase().as_str() {
        "vanilla" | "none" | "" => None,
        "fabric" => Some((LoaderKind::Fabric, loader_version.unwrap_or_default())),
        "quilt" => Some((LoaderKind::Quilt, loader_version.unwrap_or_default())),
        "forge" => Some((
            LoaderKind::Forge,
            loader_version.ok_or_else(|| anyhow::anyhow!("forge 需要 --loader-version <build>"))?,
        )),
        "neoforge" => Some((
            LoaderKind::NeoForge,
            loader_version
                .ok_or_else(|| anyhow::anyhow!("neoforge 需要 --loader-version <version>"))?,
        )),
        other => anyhow::bail!("未知 loader: {other}(用 vanilla|fabric|quilt|forge|neoforge)"),
    };

    let loader_desc = match &loader_opt {
        None => "原版".to_string(),
        Some((k, v)) if v.is_empty() => format!("{} (最新)", k.as_str()),
        Some((k, v)) => format!("{} {v}", k.as_str()),
    };
    println!("从零创建实例「{name}」(MC {mc_version} · {loader_desc}) …");

    let tx = live_progress();
    // 新实例默认内存/Java 取自全局设置(与桌面端一致)。
    let g = GlobalSettings::load(&data_dir()).unwrap_or_default();
    let id = mc_core::instance::lifecycle::create_instance(
        &dl, &paths, name, mc_version, loader_opt, g.default_memory_mb, g.java_path.clone(), Some(tx),
    )
    .await?;
    println!("✓ 已创建实例: {id}");
    println!("  启动:   mc launch {id} --name <你的名字>");
    println!("  装 mod: mc install-mod {id} <项目 slug> [--loader <loader>]");
    Ok(())
}

async fn cmd_fabric(cli: &Cli, mc_version: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let manifest = meta::fetch_manifest(&dl).await?;
    let entry = manifest
        .iter()
        .find(|v| v.id == mc_version)
        .with_context(|| format!("版本 {mc_version} 不在清单中"))?;

    println!("为 {mc_version} 安装 Fabric…");
    let (tx, mut rx) = tokio::sync::watch::channel(mc_core::types::Progress::new("准备"));
    let printer = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow().clone();
            if p.total > 0 {
                println!("  {} {}/{}", p.stage, p.current, p.total);
            } else {
                println!("  {}", p.stage);
            }
        }
    });
    let id = mc_core::loader::install_fabric(&dl, &paths, mc_version, entry, None, Some(tx)).await?;
    let _ = printer.await;
    println!("✓ Fabric 安装完成,实例 id: {id}");
    println!("  用 `mc launch {id} --name <你>` 启动");
    Ok(())
}

async fn cmd_quilt(cli: &Cli, mc_version: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let manifest = meta::fetch_manifest(&dl).await?;
    let entry = manifest
        .iter()
        .find(|v| v.id == mc_version)
        .with_context(|| format!("版本 {mc_version} 不在清单中"))?;
    println!("为 {mc_version} 安装 Quilt…");
    let id = mc_core::loader::install_quilt(&dl, &paths, mc_version, entry, None, None).await?;
    println!("✓ Quilt 安装完成,实例 id: {id}");
    Ok(())
}

fn live_progress() -> tokio::sync::watch::Sender<mc_core::types::Progress> {
    let (tx, mut rx) = tokio::sync::watch::channel(mc_core::types::Progress::new("准备"));
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow().clone();
            if p.total > 0 {
                println!("  {} {}/{}", p.stage, p.current, p.total);
            } else {
                println!("  {}", p.stage);
            }
        }
    });
    tx
}

async fn cmd_forge(cli: &Cli, mc_version: &str, forge_build: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let manifest = meta::fetch_manifest(&dl).await?;
    let entry = manifest.iter().find(|v| v.id == mc_version).with_context(|| format!("版本 {mc_version} 不在清单中"))?;
    println!("为 {mc_version} 安装 Forge {forge_build}…");
    let id = mc_core::loader::install_forge(&dl, &paths, mc_version, forge_build, entry, None, Some(live_progress())).await?;
    println!("✓ Forge 安装完成,实例 id: {id}");
    Ok(())
}

async fn cmd_neoforge(cli: &Cli, neo_version: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let mc = mc_core::loader::neoforge::mc_version_for(neo_version).context("无法推断 MC 版本")?;
    let manifest = meta::fetch_manifest(&dl).await?;
    let entry = manifest.iter().find(|v| v.id == mc).with_context(|| format!("版本 {mc} 不在清单中"))?;
    println!("安装 NeoForge {neo_version} (MC {mc})…");
    let id = mc_core::loader::install_neoforge(&dl, &paths, neo_version, entry, None, Some(live_progress())).await?;
    println!("✓ NeoForge 安装完成,实例 id: {id}");
    Ok(())
}

async fn cmd_java_install(major: u8) -> Result<()> {
    let dl = downloader(false)?;
    let dest = data_dir().join("java");
    println!("下载 Java {major} (Adoptium) 到 {}…", dest.display());
    let path = mc_core::java::install::install_jre(&dl, &dest, major).await?;
    println!("✓ Java {major} 已安装: {}", path.display());
    Ok(())
}

fn cmd_crash(path: &PathBuf) -> Result<()> {
    let log = std::fs::read_to_string(path).with_context(|| format!("读取 {}", path.display()))?;
    match mc_core::diagnostics::analyze(&log) {
        Some(a) => {
            println!("原因: {}", a.reason);
            for s in &a.suggestions {
                println!("  建议: {s}");
            }
            if let Some(m) = &a.matched {
                println!("  命中: {m}");
            }
        }
        None => println!("未识别出已知崩溃模式。"),
    }
    Ok(())
}

fn cmd_mods(cli: &Cli, id: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let inst = Instance::new(id, paths.root().to_path_buf());
    let mods = mc_core::instance::list_mods(&inst);
    if mods.is_empty() {
        println!("实例 {id} 没有 mod(或 mods 目录为空)。");
        return Ok(());
    }
    for m in mods {
        println!(
            "{} {:<40} {} [{}]",
            if m.enabled { "●" } else { "○" },
            m.name,
            m.version.as_deref().unwrap_or("-"),
            m.loader
        );
    }
    Ok(())
}

fn cmd_worlds(cli: &Cli, id: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let inst = Instance::new(id, paths.root().to_path_buf());
    let worlds = mc_core::instance::list_worlds(&inst);
    if worlds.is_empty() {
        println!("实例 {id} 没有存档。");
        return Ok(());
    }
    for w in worlds {
        println!("{:<28} {:<10} {} bytes", w.name, w.game_mode, w.size_bytes);
    }
    Ok(())
}

async fn cmd_install_mod(
    cli: &Cli,
    id: &str,
    project: &str,
    loader: &str,
    mc_version: Option<String>,
) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let inst = Instance::new(id, paths.root().to_path_buf());
    let dl = downloader(cli.mirror)?;
    let api = mc_core::modplatform::modrinth::ModrinthApi::new();
    let mc = mc_version.unwrap_or_else(|| id.to_string());

    println!("安装 {project} ({loader}, MC {mc}) 到实例 {id}…");
    let report = mc_core::instance::install_mod(&api, &dl, &inst, project, &mc, loader, true).await?;
    for m in &report.installed {
        println!("  ✓ {}", m.file_name);
    }
    if !report.unresolved.is_empty() {
        println!("  未解决依赖: {}", report.unresolved.join(", "));
    }
    println!("完成:安装 {} 个文件", report.installed.len());
    Ok(())
}

async fn cmd_loaders(mc_version: &str) -> Result<()> {
    let client = mc_core::server::ServerClient::new()?;
    println!("通过 {} 查询 {} 的加载器版本…", client.base_url(), mc_version);
    let meta = client.loaders(mc_version).await?;
    println!("  fabric:   {} 个 (最新 {})", meta.loaders.fabric.len(), meta.loaders.fabric.first().map(String::as_str).unwrap_or("-"));
    println!("  quilt:    {} 个 (最新 {})", meta.loaders.quilt.len(), meta.loaders.quilt.first().map(String::as_str).unwrap_or("-"));
    println!("  forge:    {} 个 (最新 {})", meta.loaders.forge.len(), meta.loaders.forge.first().map(String::as_str).unwrap_or("-"));
    println!("  neoforge: {} 个 (最新 {})", meta.loaders.neoforge.len(), meta.loaders.neoforge.first().map(String::as_str).unwrap_or("-"));
    Ok(())
}

async fn cmd_register_account(email: &str, password: &str) -> Result<()> {
    let client = mc_core::server::ServerClient::new()?;
    println!("在 {} 注册账号 {email}…", client.base_url());
    // name 取邮箱本地部分。
    let name = email.split('@').next().unwrap_or(email);
    let user = client.register(email, password, name).await?;
    println!("✓ 注册成功: id={} email={}", user.id, user.email.as_deref().unwrap_or("-"));
    // 同一个 client 保留了会话 cookie,get-session 无需再传 token。
    let me = client.me().await?;
    println!("✓ 会话校验: id={} email={}", me.id, me.email.as_deref().unwrap_or("-"));
    Ok(())
}

async fn cmd_server_health() -> Result<()> {
    let client = mc_core::server::ServerClient::new()?;
    println!("ping {} …", client.base_url());
    let v = client.health().await?;
    println!("{v}");
    Ok(())
}

async fn cmd_launch(
    cli: &Cli,
    id: &str,
    name: &str,
    use_account: bool,
    online: bool,
    java_path: Option<PathBuf>,
) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let accounts_path = data_dir().join("accounts.json");

    // 选中的账号在启动前续期,镜像桌面端行为,避免 >24h 的旧 session 静默掉线:
    // 微软走 refresh_token 免浏览器续期;外置(Yggdrasil)走 validate/refresh。续期失败
    // best-effort 忽略,用现有 token 继续启动(由皮肤站/会话服务器在游戏内最终把关)。
    let mut extra_jvm_args: Vec<String> = Vec::new();
    let session = if use_account {
        let mut store = AccountStore::load(&accounts_path)?;
        let _ = auth::refresh_selected_microsoft(&mut store, &msa_client(), 600).await;
        if let Err(e) = refresh_selected_yggdrasil(&mut store).await {
            eprintln!("外置登录续期失败(用现有 token 继续):{e}");
        }
        // 外置账号:下载 authlib-injector 并注入 `-javaagent`,否则外置皮肤/联机校验不生效。
        if let Some(yg_base) =
            store.selected_account().and_then(|a| a.yggdrasil_base.clone())
        {
            let jar = auth::yggdrasil::download_authlib_injector(&dl, &data_dir().join("authlib"))
                .await
                .context("下载 authlib-injector")?;
            extra_jvm_args.push(auth::yggdrasil::javaagent_arg(&jar, &yg_base));
        }
        store
            .selected_session()
            .context("没有选中的账号,先运行 `mc login`")?
    } else {
        offline_session(name)
    };

    println!("启动 {} (玩家: {}) …", id, session.username);
    let spec = LaunchSpec {
        instance: Instance::new(id, paths.root().to_path_buf()),
        session,
        java_path,
        launcher_name: LAUNCHER_NAME.to_string(),
        launcher_version: LAUNCHER_VERSION.to_string(),
        online,
        runtimes_dir: Some(data_dir().join("java")),
        global_java_path: GlobalSettings::load(&data_dir())
            .unwrap_or_default()
            .java_path
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from),
        extra_jvm_args,
        server_override: None,
    };

    let (tx, mut rx) = tokio::sync::watch::channel(mc_core::types::Progress::new("准备"));
    let printer = tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let p = rx.borrow().clone();
            println!("  {} {}/{}", p.stage, p.current, p.total);
        }
    });

    let mut child = launch::launch(spec, &dl, Some(tx)).await?;
    let _ = printer.await;
    println!("✓ 游戏进程已启动 (pid {:?})。游戏日志:", child.id());

    // Drain the child's piped stdout/stderr so the game does not block on a full
    // pipe buffer, and surface the log so we can see it actually booting.
    use tokio::io::{AsyncBufReadExt, BufReader};
    if let Some(out) = child.stdout.take() {
        let mut lines = BufReader::new(out).lines();
        tokio::spawn(async move {
            while let Ok(Some(l)) = lines.next_line().await {
                println!("[game] {l}");
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let mut lines = BufReader::new(err).lines();
        tokio::spawn(async move {
            while let Ok(Some(l)) = lines.next_line().await {
                eprintln!("[game!] {l}");
            }
        });
    }

    let status = child.wait().await.context("等待游戏进程")?;
    println!("游戏退出,状态: {status}");
    Ok(())
}

/// Build the Microsoft auth client, mirroring the desktop's resolution order so
/// CLI login uses the same Azure app id: runtime `MC_MSA_CLIENT_ID` → compile-time
/// baked id → vanilla legacy id. The application (client) id is a public
/// identifier (device-code / public-client flow uses no secret).
fn msa_client() -> MsaClient {
    let runtime = std::env::var("MC_MSA_CLIENT_ID").ok();
    let baked = option_env!("MC_MSA_CLIENT_ID").map(str::to_string);
    match runtime.or(baked).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        Some(id) => MsaClient::with_client_id(id),
        None => MsaClient::new(),
    }
}

/// 若当前选中的是外置(Yggdrasil)账号,启动前用 validate/refresh 续期并写回 store。
///
/// 先 `validate`:仍有效则直接复用现有 token(no-op);若失效(403)则用旧
/// access_token + 持久化的 client_token `refresh` 出新 token,原地更新并保持选中。
/// 非外置账号、缺少 client_token / base 时直接返回 `Ok(())`。
async fn refresh_selected_yggdrasil(store: &mut AccountStore) -> Result<()> {
    let (uuid, base, access_token, client_token) = match store.selected_account() {
        Some(a) if a.kind == AccountKind::Yggdrasil => {
            match (a.yggdrasil_base.clone(), a.client_token.clone()) {
                (Some(base), Some(ct)) => {
                    (a.uuid.clone(), base, a.access_token.clone(), ct)
                }
                // 缺 base 或 client_token(老数据)无法续期,交由游戏内校验兜底。
                _ => return Ok(()),
            }
        }
        _ => return Ok(()),
    };

    let client = YggdrasilClient::new(base.clone());
    // 仍有效就不动 token,避免无谓地让旧 token 失效。
    if client.validate(&access_token, &client_token).await? {
        return Ok(());
    }

    let refreshed = client.refresh(&access_token, &client_token).await?;
    let prev = store
        .selected_account()
        .cloned()
        .context("外置账号在续期过程中丢失")?;
    store.add(StoredAccount {
        kind: AccountKind::Yggdrasil,
        username: if refreshed.username.is_empty() { prev.username } else { refreshed.username },
        uuid: uuid.clone(),
        access_token: refreshed.access_token,
        refresh_token: None,
        xuid: String::new(),
        user_type: "msa".to_string(),
        owns_game: true,
        expires_at: None,
        client_token: Some(refreshed.client_token),
        yggdrasil_base: Some(base),
    });
    store.select(&uuid)?;
    store.save()?;
    Ok(())
}

async fn cmd_login() -> Result<()> {
    let client = msa_client();
    let code = client.device_code_start().await.context("获取设备码")?;
    println!("\n请在浏览器打开:  {}", code.verification_uri);
    println!("输入代码:        {}\n", code.user_code);
    println!("等待授权…(完成登录后自动继续)");

    let token = client.poll_token(&code.device_code, code.interval).await?;
    let session = client.authenticate(&token.access_token).await?;

    let mut store = AccountStore::load(data_dir().join("accounts.json"))?;
    store.add(StoredAccount {
        kind: AccountKind::Microsoft,
        username: session.username.clone(),
        uuid: session.uuid.clone(),
        access_token: session.access_token.clone(),
        refresh_token: Some(token.refresh_token.clone()),
        xuid: session.xuid.clone(),
        user_type: session.user_type.clone(),
        owns_game: true,
        expires_at: Some(now_unix() + MC_TOKEN_TTL_SECS),
        client_token: None,
        yggdrasil_base: None,
    });
    store.select(&session.uuid)?;
    store.save()?;
    println!("\n✓ 登录成功:{} ({})", session.username, session.uuid);
    Ok(())
}

/// 离线登录:由用户名派生稳定 UUID,落库为离线账号并选中。
fn cmd_login_offline(name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        anyhow::bail!("用户名不能为空");
    }
    let session = offline_session(name);
    let mut store = AccountStore::load(data_dir().join("accounts.json"))?;
    store.add(StoredAccount {
        kind: AccountKind::Offline,
        username: session.username.clone(),
        uuid: session.uuid.clone(),
        access_token: session.access_token,
        refresh_token: None,
        xuid: session.xuid,
        user_type: session.user_type,
        owns_game: false,
        expires_at: None,
        client_token: None,
        yggdrasil_base: None,
    });
    store.select(&session.uuid)?;
    store.save()?;
    println!("✓ 已添加离线账号:{} ({})", session.username, session.uuid);
    Ok(())
}

/// 外置(Yggdrasil)登录:用 base + 用户名 + 密码登录皮肤站,落库为外置账号并选中,
/// 持久化 client_token(续期所需)与 base(启动时注入 authlib-injector 所需)。
async fn cmd_login_yggdrasil(base: &str, username: &str, password: &str) -> Result<()> {
    let base = base.trim();
    if base.is_empty() || username.trim().is_empty() {
        anyhow::bail!("皮肤站地址和用户名不能为空");
    }
    let client = YggdrasilClient::new(base);
    println!("在 {} 外置登录 {} …", client.base(), username.trim());
    let session = client.authenticate(username.trim(), password).await?;
    let mut store = AccountStore::load(data_dir().join("accounts.json"))?;
    store.add(StoredAccount {
        kind: AccountKind::Yggdrasil,
        username: session.username.clone(),
        uuid: session.uuid.clone(),
        access_token: session.access_token,
        refresh_token: None,
        xuid: String::new(),
        user_type: "msa".to_string(),
        owns_game: true,
        expires_at: None,
        client_token: Some(session.client_token),
        yggdrasil_base: Some(client.base().to_string()),
    });
    store.select(&session.uuid)?;
    store.save()?;
    println!("\n✓ 外置登录成功:{} ({})", session.username, session.uuid);
    Ok(())
}

fn cmd_accounts() -> Result<()> {
    let store = AccountStore::load(data_dir().join("accounts.json"))?;
    let accounts = store.list();
    if accounts.is_empty() {
        println!("没有已保存的账号。运行 `mc login` 添加微软账号。");
        return Ok(());
    }
    for a in accounts {
        println!(
            "{} {:<16} {:?} {}",
            if a.selected { "*" } else { " " },
            a.username,
            a.kind,
            a.uuid
        );
    }
    Ok(())
}

// --- new-feature exercisers: content providers + modpack import/export -------

async fn cmd_search(
    query: &str,
    cf: bool,
    kind: &str,
    mc_version: Option<String>,
    loader: Option<String>,
) -> Result<()> {
    use mc_core::modplatform::provider::ResourceProvider;
    use mc_core::modplatform::{ResourceKind, SearchQuery};
    use std::sync::Arc;

    let rk = match kind {
        "modpack" => ResourceKind::Modpack,
        "shader" => ResourceKind::Shader,
        "resourcepack" => ResourceKind::ResourcePack,
        "datapack" => ResourceKind::Datapack,
        _ => ResourceKind::Mod,
    };
    let provider: Arc<dyn ResourceProvider> = if cf {
        Arc::new(
            mc_core::modplatform::curseforge::CurseForgeProvider::from_env()
                .context("CurseForge 需要环境变量 MC_CF_API_KEY")?,
        )
    } else {
        Arc::new(mc_core::modplatform::modrinth::ModrinthProvider::new())
    };

    let mut q = SearchQuery::new(query, rk);
    q.game_version = mc_version;
    q.loader = loader;
    q.limit = 10;

    println!("通过 {} 搜索「{}」…", provider.caps().readable_name, query);
    let hits = provider.search(&q).await?;
    if hits.is_empty() {
        println!("(无结果)");
    }
    for h in hits {
        println!("  {:<24} {:<32} ⬇ {}", h.slug, h.title, h.downloads);
    }
    Ok(())
}

async fn cmd_resolve_hash(sha512: &str) -> Result<()> {
    use mc_core::modplatform::provider::ResourceProvider;
    use mc_core::modplatform::HashAlgo;

    let provider = mc_core::modplatform::modrinth::ModrinthProvider::new();
    let hashes = vec![sha512.to_string()];
    println!("在 Modrinth 按 sha512 反查 {sha512} …");
    let res = provider.resolve_by_hashes(HashAlgo::Sha512, &hashes).await?;
    match res.into_iter().next().flatten() {
        Some(r) => println!(
            "✓ 命中: project={} version={} file={}\n  url={}",
            r.project_id, r.version_id, r.file.filename, r.file.url
        ),
        None => println!("✗ Modrinth 未收录该 sha512"),
    }
    Ok(())
}

/// Show or mutate the global launcher settings — the daemon state shared with
/// the desktop UI. `mc settings` prints; `mc settings set --…` persists changes.
fn cmd_settings(action: &Option<SettingsAction>) -> Result<()> {
    let dir = data_dir();
    let mut s = GlobalSettings::load(&dir)?;

    if let Some(SettingsAction::Set {
        download_source,
        concurrency,
        memory_mb,
        java_path,
        use_mirror,
        language,
    }) = action
    {
        if let Some(v) = download_source {
            s.download_source = v.clone();
        }
        if let Some(v) = concurrency {
            s.concurrency = (*v).max(1);
        }
        if let Some(v) = memory_mb {
            s.default_memory_mb = *v;
        }
        if let Some(v) = java_path {
            // 空串 = 清空 → 自动检测。
            s.java_path = if v.is_empty() { None } else { Some(v.clone()) };
        }
        if let Some(v) = use_mirror {
            s.use_mirror = *v;
        }
        if let Some(v) = language {
            s.language = v.clone();
        }
        s.save(&dir).context("写入 settings.json")?;
        println!("✓ 已保存到 {}\n", GlobalSettings::path(&dir).display());
    }

    print_settings(&dir, &s);
    Ok(())
}

/// Pretty-print the settings exactly as the daemon holds them, plus the derived
/// mirror decision (the part that actually drives downloads).
fn print_settings(dir: &std::path::Path, s: &GlobalSettings) {
    let wants_mirror = s.use_mirror || s.download_source.eq_ignore_ascii_case("bmclapi");
    println!("settings.json  ({})", GlobalSettings::path(dir).display());
    println!("  download_source : {}", s.download_source);
    println!("  concurrency     : {}", s.concurrency);
    println!("  default_memory  : {} MiB", s.default_memory_mb);
    println!("  java_path       : {}", s.java_path.as_deref().unwrap_or("(自动检测)"));
    println!("  use_mirror      : {}", s.use_mirror);
    println!("  language        : {}", s.language);
    println!("  server_url      : {}", s.server_url.as_deref().unwrap_or("-"));
    println!(
        "  custom_roots    : {}",
        if s.custom_roots.is_empty() { "-".to_string() } else { s.custom_roots.join(", ") }
    );
    println!("  → 下载走镜像     : {}", if wants_mirror { "是 (BMCLAPI + McIM)" } else { "否 (官方直连)" });
}

async fn cmd_project(id: &str) -> Result<()> {
    let api = mc_core::modplatform::modrinth::ModrinthApi::new();
    println!("拉取项目 {id} 详情…");
    let p = api.project_details(id).await?;
    println!("\n{}  ({})", p.title, p.slug);
    println!("  {}", p.description);
    println!("  ⬇ {}  ♥ {}", p.downloads, p.followers);
    if !p.categories.is_empty() {
        println!("  分类: {}", p.categories.join(", "));
    }
    let links: Vec<(&str, &Option<String>)> = vec![
        ("源码", &p.source_url),
        ("问题", &p.issues_url),
        ("Wiki", &p.wiki_url),
        ("Discord", &p.discord_url),
    ];
    for (label, url) in links {
        if let Some(u) = url {
            println!("  {label}: {u}");
        }
    }
    println!("  画廊: {} 张图", p.gallery.len());
    for g in &p.gallery {
        println!("    - {}{}", g.url, g.title.as_deref().map(|t| format!("  ({t})")).unwrap_or_default());
    }
    println!("  简介正文: {} 字符 (markdown)", p.body.chars().count());
    // 打印正文前若干行,便于在终端核对内容。
    let preview: String = p.body.lines().take(12).collect::<Vec<_>>().join("\n");
    if !preview.trim().is_empty() {
        println!("  ----- body 预览 -----\n{preview}\n  ---------------------");
    }
    Ok(())
}

fn cmd_modpack_detect(file: &Path) -> Result<()> {
    use mc_core::modpack::import::archive::PackArchive;
    use mc_core::modpack::import::ImportEngine;
    use mc_core::modplatform::provider::ProviderRegistry;

    // PreparedIndex 预取 manifest 字节,使 detect() 的内容判别(CF vs MCBBS)可用。
    // PackArchive 同时支持 zip 文件与未解压的实例目录。
    let idx = PackArchive::open(file)?.into_prepared(&["manifest.json", "mcbbs.packmeta"]);
    let engine = ImportEngine::with_defaults(Downloader::new(4)?, ProviderRegistry::with_defaults());
    match engine.dispatch(&idx) {
        Some((_, m)) => println!(
            "✓ 识别格式: {}  (包根 '{}',置信度 {})",
            m.format, m.archive_root, m.confidence
        ),
        None => println!("✗ 无法识别的整合包格式"),
    }
    Ok(())
}

async fn cmd_modpack_import(cli: &Cli, file: &Path, id: Option<String>) -> Result<()> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};
    use mc_core::modplatform::provider::ProviderRegistry;

    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let engine = ImportEngine::with_defaults(dl, ProviderRegistry::with_defaults());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = id;

    println!("导入整合包 {} 到 {} …", file.display(), paths.root().display());
    let out = engine.import(ImportSource::LocalFile(file.to_path_buf()), opts).await?;
    println!("✓ 已建实例: {}", out.instance_id);
    if !out.blocked.is_empty() {
        println!("  {} 个文件需手动下载(CurseForge 禁第三方分发):", out.blocked.len());
        for b in &out.blocked {
            println!("    - {}  ({})", b.name, b.website_url);
        }
    }
    if !out.skipped_optional.is_empty() {
        println!("  跳过 {} 个可选文件", out.skipped_optional.len());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_modpack_export(
    cli: &Cli,
    id: &str,
    target: &str,
    out: Option<PathBuf>,
    name: Option<String>,
    mc_version: Option<String>,
    loader: Option<String>,
    loader_version: Option<String>,
) -> Result<()> {
    use mc_core::modpack::export::{
        CurseForgeExportTarget, ExportInput, ExportTarget, ModListExportTarget, ModListFormat,
        ModpackExporter, ModrinthExportTarget,
    };
    use mc_core::types::LoaderKind;

    let paths = resolve_root(&cli.dir);
    let inst = Instance::new(id, paths.root().to_path_buf());
    let game_root = inst.game_dir();

    // 缺省的 name / mc_version 从实例摘要补全。
    let summary = mc_core::instance::list_instances(&paths).into_iter().find(|i| i.id == id);
    let name = name.unwrap_or_else(|| id.to_string());
    let mc = mc_version
        .or_else(|| summary.as_ref().map(|s| s.mc_version.clone()))
        .unwrap_or_else(|| id.to_string());

    let (kind, sub) = target.split_once(':').unwrap_or((target, ""));
    let mr = ModrinthExportTarget::new();
    let cf = CurseForgeExportTarget::new();
    let ml = ModListExportTarget::new(match sub {
        "html" => ModListFormat::Html,
        "json" => ModListFormat::Json,
        "csv" => ModListFormat::Csv,
        "txt" => ModListFormat::PlainText,
        _ => ModListFormat::Markdown,
    });
    let tgt: &dyn ExportTarget = match kind {
        "modrinth" => &mr,
        "curseforge" => &cf,
        "modlist" => &ml,
        other => anyhow::bail!("未知导出目标: {other}"),
    };

    let mut input = ExportInput::new(&game_root, name, mc);
    // loader:命令行优先,否则用实例摘要。
    let loader = loader.or_else(|| summary.as_ref().map(|s| s.loader.as_str().to_string()));
    let loader_version =
        loader_version.or_else(|| summary.as_ref().and_then(|s| s.loader_version.clone()));
    if let (Some(k), Some(v)) = (loader.as_deref(), loader_version) {
        let lk = match k.to_ascii_lowercase().as_str() {
            "forge" => Some(LoaderKind::Forge),
            "neoforge" => Some(LoaderKind::NeoForge),
            "fabric" => Some(LoaderKind::Fabric),
            "quilt" => Some(LoaderKind::Quilt),
            "liteloader" => Some(LoaderKind::LiteLoader),
            "optifine" => Some(LoaderKind::OptiFine),
            _ => None,
        };
        if let Some(lk) = lk {
            if !v.is_empty() {
                input.loader = Some((lk, v));
            }
        }
    }

    println!("导出实例 {id} → {kind}{} …", if sub.is_empty() { String::new() } else { format!(":{sub}") });
    let exporter = ModpackExporter::with_defaults();
    let dest = exporter.export(tgt, input, &mut |_, _, _| {}).await?;
    let final_dest = match out {
        Some(o) => {
            if std::fs::rename(&dest, &o).is_err() {
                std::fs::copy(&dest, &o)?;
                let _ = std::fs::remove_file(&dest);
            }
            o
        }
        None => dest,
    };
    println!("✓ 已导出: {}", final_dest.display());
    Ok(())
}
