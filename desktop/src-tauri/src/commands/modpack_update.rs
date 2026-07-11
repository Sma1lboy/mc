use super::*;

/// 检查某实例(由 Modrinth 整合包安装)是否有更新:返回比当前来源版本更新的版本列表。
/// 非整合包来源 / 非 modrinth / 缺 project_id 时返回空(前端据此不显示更新提示)。
#[tauri::command]
#[specta::specta]
pub async fn check_modpack_updates(
    root: String,
    id: String,
) -> CmdResult<Vec<mc_core::modplatform::modrinth::VersionDetail>> {
    let inst = instance_of(&root, &id);
    let Some(src) = inst.load_config().map_err(err)?.source else {
        return Ok(Vec::new());
    };
    if src.provider != "modrinth" {
        return Ok(Vec::new());
    }
    let all = ModrinthApi::new().version_details(&src.project_id).await.map_err(err)?;
    Ok(mc_core::modpack::update::newer_versions(all, src.version_id.as_deref()))
}

/// 一次性检查 `root` 下所有实例的可用更新(每实例:mod 更新数 + 整合包是否有新版)。
/// 网络密集,前端仅按需调用;内部有界并发推进,单实例失败被跳过不影响整批。
/// 只返回**至少有一项更新**的实例,前端据此点亮卡片角标。
#[tauri::command]
#[specta::specta]
pub async fn check_all_updates(root: String) -> CmdResult<Vec<mc_core::instance::InstanceUpdateInfo>> {
    let paths = root_paths(&root);
    let api = ModrinthApi::new();
    Ok(mc_core::instance::check_all_updates(&api, &paths).await)
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
    let tx = progress_channel(app, "install://progress", "准备更新");
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

/// 取一个项目的完整详情(简介标签页用:长描述正文 + 画廊 + 关注数 + 源码/issue/wiki 等
/// 外部链接)。provider 感知(缺省 `modrinth`):CurseForge 走 Flame 元信息 + description
/// 端点,映射成同一份 `ProjectDetail`,前端渲染不感知平台。
#[tauri::command]
#[specta::specta]
pub async fn modrinth_project(
    project_id: String,
    provider: Option<String>,
) -> CmdResult<mc_core::modplatform::modrinth::ProjectDetail> {
    use mc_core::modplatform::ProviderId;
    // 走本地持久缓存:实例详情头部 + 概览每次打开都要这份数据,缓存 24h 避免每次都打平台
    // (抓取失败时回退旧缓存,离线也能显示)。
    let cache = data_dir().join("cache");
    let ttl = std::time::Duration::from_secs(24 * 3600);
    match parse_provider(provider.as_deref())? {
        ProviderId::Modrinth => ModrinthApi::new()
            .project_details_cached(&project_id, &cache, ttl)
            .await
            .map_err(err),
        ProviderId::CurseForge => {
            let key = settings_global()
                .resolved_cf_api_key()
                .ok_or_else(|| "CurseForge 未配置 API Key".to_string())?;
            let id: i64 = project_id
                .parse()
                .map_err(|_| format!("非法的 CurseForge 项目 id: {project_id}"))?;
            mc_core::modplatform::curseforge::FlameApi::new(key)
                .project_details_cached(id, &cache, ttl)
                .await
                .map_err(err)
        }
    }
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
    let tx = progress_channel(app, "install://progress", "准备");
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
    let tx = progress_channel(app, "install://progress", "准备");
    let outcome = engine
        .import_with_progress(ImportSource::Url(url), opts, Some(tx))
        .await
        .map_err(err)?;
    Ok(outcome.into())
}

