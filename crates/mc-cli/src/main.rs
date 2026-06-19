//! `mc` — a headless CLI front-end over `mc-core`. It exists to drive and verify
//! the engine without any UI (milestone M3 in `docs/04`): list versions, install
//! a version, log in, and launch the game.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use mc_core::auth::{offline_session, AccountStore, MsaClient, StoredAccount};
use mc_core::download::{Downloader, MirrorResolver};
use mc_core::instance::Instance;
use mc_core::launch::{self, LaunchSpec};
use mc_core::paths::{self, GamePaths};
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
    /// List stored accounts.
    Accounts,
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

/// Resolve the game root to operate on.
fn resolve_root(dir: &Option<PathBuf>) -> GamePaths {
    if let Some(d) = dir {
        return GamePaths::new(d.clone());
    }
    let roots = paths::discover_roots(&exe_dir(), &data_dir(), &[]);
    let path = roots
        .first()
        .map(|r| PathBuf::from(&r.path))
        .unwrap_or_else(|| data_dir().join(".minecraft"));
    GamePaths::new(path)
}

fn downloader(mirror: bool) -> Result<Downloader> {
    let dl = Downloader::new(64).context("building downloader")?;
    Ok(if mirror { dl.with_mirror(MirrorResolver::bmclapi()) } else { dl })
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
        Command::Accounts => cmd_accounts(),
    }
}

fn cmd_roots() -> Result<()> {
    let roots = paths::discover_roots(&exe_dir(), &data_dir(), &[]);
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
            "Java {}  ({}{})  {}",
            j.version,
            if j.is_64bit { "64-bit" } else { "32-bit" },
            format!(", {}", j.source),
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
    let id = mc_core::loader::install_fabric(&dl, &paths, mc_version, entry, Some(tx)).await?;
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
    let id = mc_core::loader::install_quilt(&dl, &paths, mc_version, entry, None).await?;
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

    let session = if use_account {
        let store = AccountStore::load(data_dir().join("accounts.json"))?;
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

async fn cmd_login() -> Result<()> {
    let client = MsaClient::new();
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
    });
    store.select(&session.uuid)?;
    store.save()?;
    println!("\n✓ 登录成功:{} ({})", session.username, session.uuid);
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
