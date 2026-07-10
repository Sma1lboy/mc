use super::*;

pub(crate) fn cmd_roots() -> Result<()> {
    let roots = paths::discover_roots(&exe_dir(), &data_dir(), &custom_roots());
    if roots.is_empty() {
        println!("(没有发现游戏根目录)");
    }
    for r in roots {
        println!("[{:?}] {}  ->  {}", r.kind, r.name, r.path);
    }
    Ok(())
}

pub(crate) async fn cmd_java() -> Result<()> {
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

pub(crate) async fn cmd_versions(cli: &Cli, snapshot: bool, limit: usize) -> Result<()> {
    let dl = downloader(cli.mirror)?;
    let versions = meta::fetch_manifest(&dl)
        .await
        .context("fetching version manifest")?;
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

pub(crate) fn cmd_list(cli: &Cli) -> Result<()> {
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

pub(crate) async fn cmd_install(cli: &Cli, id: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    println!("正在拉取版本清单…");
    let entry = meta::resolve_version(&dl, id).await.with_context(|| format!("版本 {id} 不在清单中"))?;
    let entry = &entry;

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

pub(crate) async fn cmd_create(
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
        &dl,
        &paths,
        name,
        mc_version,
        loader_opt,
        g.default_memory_mb,
        g.java_path.clone(),
        Some(tx),
    )
    .await?;
    println!("✓ 已创建实例: {id}");
    println!("  启动:   mc launch {id} --name <你的名字>");
    println!("  装 mod: mc install-mod {id} <项目 slug> [--loader <loader>]");
    Ok(())
}

pub(crate) async fn cmd_fabric(cli: &Cli, mc_version: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let entry = meta::resolve_version(&dl, mc_version)
        .await
        .with_context(|| format!("版本 {mc_version} 不在清单中"))?;
    let entry = &entry;

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

pub(crate) async fn cmd_quilt(cli: &Cli, mc_version: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let entry = meta::resolve_version(&dl, mc_version)
        .await
        .with_context(|| format!("版本 {mc_version} 不在清单中"))?;
    let entry = &entry;
    println!("为 {mc_version} 安装 Quilt…");
    let id = mc_core::loader::install_quilt(&dl, &paths, mc_version, entry, None, None).await?;
    println!("✓ Quilt 安装完成,实例 id: {id}");
    Ok(())
}

pub(crate) fn live_progress() -> tokio::sync::watch::Sender<mc_core::types::Progress> {
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

pub(crate) async fn cmd_forge(cli: &Cli, mc_version: &str, forge_build: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let entry = meta::resolve_version(&dl, mc_version)
        .await
        .with_context(|| format!("版本 {mc_version} 不在清单中"))?;
    println!("为 {mc_version} 安装 Forge {forge_build}…");
    let id = mc_core::loader::install_forge(&dl, &paths, mc_version, forge_build, &entry, None, Some(live_progress())).await?;
    println!("✓ Forge 安装完成,实例 id: {id}");
    Ok(())
}

pub(crate) async fn cmd_neoforge(cli: &Cli, neo_version: &str) -> Result<()> {
    let paths = resolve_root(&cli.dir);
    let dl = downloader(cli.mirror)?;
    let mc = mc_core::loader::neoforge::mc_version_for(neo_version).context("无法推断 MC 版本")?;
    let entry = meta::resolve_version(&dl, &mc)
        .await
        .with_context(|| format!("版本 {mc} 不在清单中"))?;
    println!("安装 NeoForge {neo_version} (MC {mc})…");
    let id = mc_core::loader::install_neoforge(&dl, &paths, neo_version, &entry, None, Some(live_progress())).await?;
    println!("✓ NeoForge 安装完成,实例 id: {id}");
    Ok(())
}

pub(crate) async fn cmd_java_install(major: u8) -> Result<()> {
    let dl = downloader(false)?;
    let dest = data_dir().join("java");
    println!("下载 Java {major} (Adoptium) 到 {}…", dest.display());
    let path = mc_core::java::install::install_jre(&dl, &dest, major).await?;
    println!("✓ Java {major} 已安装: {}", path.display());
    Ok(())
}

pub(crate) fn cmd_crash(path: &PathBuf) -> Result<()> {
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

pub(crate) fn cmd_mods(cli: &Cli, id: &str) -> Result<()> {
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

pub(crate) fn cmd_worlds(cli: &Cli, id: &str) -> Result<()> {
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

pub(crate) async fn cmd_install_mod(
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
    let report =
        mc_core::instance::install_mod(&api, &dl, &inst, project, &mc, loader, true).await?;
    for m in &report.installed {
        println!("  ✓ {}", m.file_name);
    }
    if !report.unresolved.is_empty() {
        println!("  未解决依赖: {}", report.unresolved.join(", "));
    }
    println!("完成:安装 {} 个文件", report.installed.len());
    Ok(())
}

pub(crate) async fn cmd_loaders(mc_version: &str) -> Result<()> {
    let client = mc_core::server::ServerClient::new()?;
    println!(
        "通过 {} 查询 {} 的加载器版本…",
        client.base_url(),
        mc_version
    );
    let meta = client.loaders(mc_version).await?;
    println!(
        "  fabric:   {} 个 (最新 {})",
        meta.loaders.fabric.len(),
        meta.loaders
            .fabric
            .first()
            .map(String::as_str)
            .unwrap_or("-")
    );
    println!(
        "  quilt:    {} 个 (最新 {})",
        meta.loaders.quilt.len(),
        meta.loaders
            .quilt
            .first()
            .map(String::as_str)
            .unwrap_or("-")
    );
    println!(
        "  forge:    {} 个 (最新 {})",
        meta.loaders.forge.len(),
        meta.loaders
            .forge
            .first()
            .map(String::as_str)
            .unwrap_or("-")
    );
    println!(
        "  neoforge: {} 个 (最新 {})",
        meta.loaders.neoforge.len(),
        meta.loaders
            .neoforge
            .first()
            .map(String::as_str)
            .unwrap_or("-")
    );
    Ok(())
}
