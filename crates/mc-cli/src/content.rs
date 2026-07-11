use super::*;

// --- new-feature exercisers: content providers + modpack import/export -------

pub(crate) async fn cmd_search(
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

pub(crate) async fn cmd_resolve_hash(sha512: &str) -> Result<()> {
    use mc_core::modplatform::provider::ResourceProvider;
    use mc_core::modplatform::HashAlgo;

    let provider = mc_core::modplatform::modrinth::ModrinthProvider::new();
    let hashes = vec![sha512.to_string()];
    println!("在 Modrinth 按 sha512 反查 {sha512} …");
    let res = provider
        .resolve_by_hashes(HashAlgo::Sha512, &hashes)
        .await?;
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
pub(crate) fn cmd_settings(action: &Option<SettingsAction>) -> Result<()> {
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
pub(crate) fn print_settings(dir: &std::path::Path, s: &GlobalSettings) {
    let wants_mirror = s.use_mirror || s.download_source.eq_ignore_ascii_case("bmclapi");
    println!("settings.json  ({})", GlobalSettings::path(dir).display());
    println!("  download_source : {}", s.download_source);
    println!("  concurrency     : {}", s.concurrency);
    println!("  default_memory  : {} MiB", s.default_memory_mb);
    println!(
        "  java_path       : {}",
        s.java_path.as_deref().unwrap_or("(自动检测)")
    );
    println!("  use_mirror      : {}", s.use_mirror);
    println!("  language        : {}", s.language);
    println!(
        "  server_url      : {}",
        s.server_url.as_deref().unwrap_or("-")
    );
    println!(
        "  custom_roots    : {}",
        if s.custom_roots.is_empty() {
            "-".to_string()
        } else {
            s.custom_roots.join(", ")
        }
    );
    println!(
        "  → 下载走镜像     : {}",
        if wants_mirror {
            "是 (BMCLAPI + McIM)"
        } else {
            "否 (官方直连)"
        }
    );
}

pub(crate) async fn cmd_project(id: &str) -> Result<()> {
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
        println!(
            "    - {}{}",
            g.url,
            g.title
                .as_deref()
                .map(|t| format!("  ({t})"))
                .unwrap_or_default()
        );
    }
    println!("  简介正文: {} 字符 (markdown)", p.body.chars().count());
    // 打印正文前若干行,便于在终端核对内容。
    let preview: String = p.body.lines().take(12).collect::<Vec<_>>().join("\n");
    if !preview.trim().is_empty() {
        println!("  ----- body 预览 -----\n{preview}\n  ---------------------");
    }
    Ok(())
}

pub(crate) fn cmd_modpack_detect(file: &Path) -> Result<()> {
    use mc_core::modpack::import::archive::PackArchive;
    use mc_core::modpack::import::ImportEngine;

    // PreparedIndex 预取 manifest 字节,使 detect() 的内容判别(CF vs MCBBS)可用。
    // PackArchive 同时支持 zip 文件与未解压的实例目录。
    let idx = PackArchive::open(file)?.into_prepared(&["manifest.json", "mcbbs.packmeta"]);
    let settings = GlobalSettings::load(&data_dir()).unwrap_or_default();
    let engine = ImportEngine::with_defaults(settings.downloader()?, settings.provider_registry());
    match engine.dispatch(&idx) {
        Some((_, m)) => println!(
            "✓ 识别格式: {}  (包根 '{}',置信度 {})",
            m.format, m.archive_root, m.confidence
        ),
        None => println!("✗ 无法识别的整合包格式"),
    }
    Ok(())
}

pub(crate) async fn cmd_modpack_import(cli: &Cli, file: &Path, id: Option<String>) -> Result<()> {
    use mc_core::modpack::import::{ImportEngine, ImportOptions, ImportSource};

    let paths = resolve_root(&cli.dir);
    let settings = GlobalSettings::load(&data_dir()).unwrap_or_default();
    let dl = downloader(cli.mirror)?;
    // 用带 CF key 的注册表:否则 CurseForge 整合包导入会拿到未注册的 CF provider,
    // resolve 直接报「需配置 API key」。与下载器的 x-api-key 配套。
    let engine = ImportEngine::with_defaults(dl, settings.provider_registry());
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = id;

    println!("导入整合包 {} 到 {} …", file.display(), paths.root().display());
    let out = engine.import(ImportSource::LocalFile(file.to_path_buf()), opts).await?;
    println!("✓ 已建实例: {}", out.instance_id);
    if !out.blocked.is_empty() {
        println!(
            "  {} 个文件需手动下载(CurseForge 禁第三方分发):",
            out.blocked.len()
        );
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
pub(crate) async fn cmd_modpack_export(
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
    let summary = mc_core::instance::list_instances(&paths)
        .into_iter()
        .find(|i| i.id == id);
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

    println!(
        "导出实例 {id} → {kind}{} …",
        if sub.is_empty() {
            String::new()
        } else {
            format!(":{sub}")
        }
    );
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
