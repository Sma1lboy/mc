use super::*;

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

    let tx = progress_channel(app, "install://progress", "准备");
    let outcome = engine
        .import_with_progress(ImportSource::LocalFile(PathBuf::from(path)), opts, Some(tx))
        .await
        .map_err(err)?;

    let dto = ImportOutcomeDto::from(outcome);
    best_effort_refresh_wiki_cache(&paths, &dto.instance_id).await;
    Ok(dto)
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
    let tx = progress_channel(app, "install://progress", "准备");
    let outcome = engine
        .import_with_progress(ImportSource::Url(url), opts, Some(tx))
        .await
        .map_err(err)?;

    let dto = ImportOutcomeDto::from(outcome);
    best_effort_refresh_wiki_cache(&paths, &dto.instance_id).await;
    Ok(dto)
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
