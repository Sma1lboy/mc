use super::*;

/// 列出某实例 mods 目录里的 mod(含启停态)。
#[tauri::command]
#[specta::specta]
pub async fn instance_mods(root: String, id: String) -> CmdResult<Vec<mc_core::instance::ModInfo>> {
    let inst = instance_of(&root, &id);
    Ok(mc_core::instance::list_mods(&inst))
}

/// 启用/停用一个 mod(改 `.jar` ↔ `.jar.disabled`)。file_name 为 list 返回的稳定标识。
#[tauri::command]
#[specta::specta]
pub fn set_mod_enabled(root: String, id: String, file_name: String, enabled: bool) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::mods::set_mod_enabled(&inst, &file_name, enabled).map_err(err)
}

/// 删除一个 mod 文件。
#[tauri::command]
#[specta::specta]
pub fn delete_mod(root: String, id: String, file_name: String) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::mods::delete_mod(&inst, &file_name).map_err(err)
}

/// 「装最新版」mod 的结果:沿用核心的依赖解析报告,再带上需手动下载的 blocked 文件
/// (CurseForge 作者禁第三方分发时)。`blocked` 非空时前端弹 BlockedFilesDialog 引导手动下。
#[derive(Default, Serialize, specta::Type)]
pub struct ModInstallReport {
    pub installed: Vec<mc_core::instance::install_mod::InstalledMod>,
    pub satisfied: Vec<String>,
    pub unresolved: Vec<String>,
    #[serde(default)]
    pub incompatible: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<BlockedFileDto>,
}

impl From<mc_core::instance::InstallReport> for ModInstallReport {
    fn from(r: mc_core::instance::InstallReport) -> Self {
        ModInstallReport {
            installed: r.installed,
            satisfied: r.satisfied,
            unresolved: r.unresolved,
            incompatible: r.incompatible,
            blocked: Vec::new(),
        }
    }
}

/// 把一个 mod 的最新兼容版本装进实例。`provider` 缺省 `modrinth`:
/// - Modrinth 走核心的「装最新版 + 解析 required 依赖」路径。
/// - CurseForge 经 provider 取最新兼容版本直接落盘(CF 文件级不带依赖,故不解析);
///   遇作者禁分发的文件经 `blocked` 回传,前端走手动下载流而非假装成功。
#[tauri::command]
#[specta::specta]
pub async fn install_mod(
    root: String,
    id: String,
    project: String,
    mc_version: String,
    loader: String,
    provider: Option<String>,
) -> CmdResult<ModInstallReport> {
    let inst = instance_of(&root, &id);
    let dl = make_downloader()?;
    match parse_provider(provider.as_deref())? {
        mc_core::modplatform::ProviderId::Modrinth => {
            let api = ModrinthApi::new();
            mc_core::instance::install_mod(&api, &dl, &inst, &project, &mc_version, &loader, true)
                .await
                .map(ModInstallReport::from)
                .map_err(err)
        }
        id @ mc_core::modplatform::ProviderId::CurseForge => {
            let p = provider_or_err(&make_registry(), id)?;
            let versions = p
                .list_versions(&project, Some(&mc_version), Some(&loader))
                .await
                .map_err(err)?;
            let v = mc_core::instance::install_mod::pick_version(&versions, &mc_version, &loader)
                .or_else(|| versions.first())
                .ok_or_else(|| format!("项目 {project} 没有兼容 {mc_version}/{loader} 的版本"))?;
            let Some(file) = v.primary_file() else {
                return Err(format!("版本 {} 没有可下载文件", v.id));
            };
            if file.url.is_empty() {
                return Ok(ModInstallReport {
                    blocked: vec![cf_blocked_dto(&project, &v.id, &file.filename, "mods")],
                    ..Default::default()
                });
            }
            let file_name = mc_core::instance::install_mod::install_mod_version(&inst, &dl, v)
                .await
                .map_err(err)?;
            Ok(ModInstallReport {
                installed: vec![mc_core::instance::install_mod::InstalledMod {
                    project_id: project,
                    file_name,
                }],
                ..Default::default()
            })
        }
    }
}

/// 显式选版安装的结果:落盘主文件 + (仅 mod)依赖解析摘要 + 需手动下载的 blocked 文件。
#[derive(Default, Serialize, specta::Type)]
pub struct VersionInstallReport {
    /// 主文件落盘名(被 blocked 时为空)。
    file: String,
    /// 新装入的 required 依赖数量(仅 mod;packs 恒为 0)。
    installed_deps: usize,
    /// 找不到兼容版本、未能解决的 required 依赖 project_id(仅 mod)。
    unresolved: Vec<String>,
    /// 所装版本声明为不兼容的项目 project_id(冲突;仅 mod)。
    #[serde(default)]
    incompatible: Vec<String>,
    /// CurseForge 作者禁第三方分发时需用户手动下载的文件;非空时前端弹 BlockedFilesDialog。
    #[serde(default)]
    blocked: Vec<BlockedFileDto>,
}

/// `target` → 包类型 + blocked 引导用的落盘目录名。
fn pack_kind_for(target: &str) -> CmdResult<(mc_core::instance::PackKind, &'static str)> {
    use mc_core::instance::PackKind;
    Ok(match target {
        "resourcepack" => (PackKind::ResourcePack, "resourcepacks"),
        "shader" => (PackKind::Shader, "shaderpacks"),
        "datapack" => (PackKind::Datapack, "datapacks"),
        other => return Err(format!("不支持的安装目标: {other}")),
    })
}

/// 安装一个**指定版本**(by version id)到实例对应位置。`provider` 缺省 `modrinth`,
/// `project` 是该版本所属项目 id(CurseForge 经 `get_files_bulk` 反查需要,Modrinth 可空)。
/// target = "mod" / "resourcepack" / "shader" / "datapack"。
///
/// mod(仅 Modrinth):在装入所选版本的同时解析它声明的 required 依赖(取各依赖最新兼容版本),
/// 与「装最新版」一致 —— 避免选版安装出一个缺前置、进不去游戏的孤立 jar。需要 `mc_version` +
/// `loader` 才能给依赖挑兼容版本;缺省时退回只装主文件。packs 与 CurseForge 不涉及依赖。
/// CurseForge 作者禁分发的文件经 `blocked` 回传,前端走手动下载流而非假装成功。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub async fn install_version_file(
    root: String,
    id: String,
    target: String,
    version_id: String,
    mc_version: Option<String>,
    loader: Option<String>,
    world: Option<String>,
    provider: Option<String>,
    project: Option<String>,
) -> CmdResult<VersionInstallReport> {
    let inst = instance_of(&root, &id);
    let dl = make_downloader()?;
    let w = world.as_deref();

    let pack_report = |file: String| VersionInstallReport { file, ..Default::default() };

    let (v, is_modrinth) = match parse_provider(provider.as_deref())? {
        mc_core::modplatform::ProviderId::Modrinth => {
            (ModrinthApi::new().get_version(&version_id).await.map_err(err)?, true)
        }
        id @ mc_core::modplatform::ProviderId::CurseForge => {
            let project = project
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "CurseForge 安装需要项目 id".to_string())?;
            let p = provider_or_err(&make_registry(), id)?;
            let mut files = p
                .get_files_bulk(&[(project.to_string(), version_id.clone())])
                .await
                .map_err(err)?;
            let resolved = files
                .pop()
                .ok_or_else(|| format!("CurseForge 版本 {version_id} 不存在"))?;
            // 禁第三方分发 → url 为空串:走与导入相同的 blocked 流,绝不假装成功。
            if resolved.file.url.is_empty() {
                let dir = if target == "mod" { "mods" } else { pack_kind_for(&target)?.1 };
                return Ok(VersionInstallReport {
                    blocked: vec![cf_blocked_dto(project, &version_id, &resolved.file.filename, dir)],
                    ..Default::default()
                });
            }
            // 把解析出的文件包成一个单文件 ProjectVersion 喂给与平台无关的落盘函数。
            let v = mc_core::modplatform::ProjectVersion {
                id: resolved.version_id,
                name: resolved.file.filename.clone(),
                version_number: resolved.file.filename.clone(),
                game_versions: Vec::new(),
                loaders: Vec::new(),
                files: vec![resolved.file],
                dependencies: Vec::new(),
                client_side: mc_core::modplatform::ProjectSideSupport::Unknown,
                server_side: mc_core::modplatform::ProjectSideSupport::Unknown,
            };
            (v, false)
        }
    };

    match target.as_str() {
        // CurseForge 文件级不带依赖模型 → 只装主文件;Modrinth 且给了 mc/loader 才解析依赖。
        "mod" => match (is_modrinth, mc_version.as_deref(), loader.as_deref()) {
            (true, Some(mc), Some(ld)) => {
                let api = ModrinthApi::new();
                let report =
                    mc_core::instance::install_mod_version_with_deps(&api, &dl, &inst, &v, mc, ld)
                        .await
                        .map_err(err)?;
                // 主文件是 installed 里 project_id 为空的那条;其余即新装的依赖。
                let file = report
                    .installed
                    .iter()
                    .find(|m| m.project_id.is_empty())
                    .map(|m| m.file_name.clone())
                    .unwrap_or_default();
                let installed_deps =
                    report.installed.iter().filter(|m| !m.project_id.is_empty()).count();
                Ok(VersionInstallReport {
                    file,
                    installed_deps,
                    unresolved: report.unresolved,
                    incompatible: report.incompatible,
                    ..Default::default()
                })
            }
            _ => mc_core::instance::install_mod_version(&inst, &dl, &v)
                .await
                .map(pack_report)
                .map_err(err),
        },
        other => {
            let (kind, _) = pack_kind_for(other)?;
            mc_core::instance::packs::install_pack_version(&inst, &dl, kind, &v, w)
                .await
                .map(pack_report)
                .map_err(err)
        }
    }
}

/// 检查实例里已启用 mod 的更新(对每个 jar 的 sha1 问 Modrinth 当前 loader/版本下的最新版)。
#[tauri::command]
#[specta::specta]
pub async fn check_mod_updates(
    root: String,
    id: String,
    mc_version: String,
    loader: String,
) -> CmdResult<Vec<mc_core::instance::ModUpdate>> {
    let inst = instance_of(&root, &id);
    let api = ModrinthApi::new();
    mc_core::instance::check_mod_updates(&api, &inst, &mc_version, &loader)
        .await
        .map_err(err)
}

/// 应用一个 mod 更新:下载新版本进 mods/ 并删掉旧文件。update 为 check_mod_updates 返回的条目。
#[tauri::command]
#[specta::specta]
pub async fn apply_mod_update(
    root: String,
    id: String,
    update: mc_core::instance::ModUpdate,
) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    let dl = make_downloader()?;
    mc_core::instance::apply_mod_update(&inst, &dl, &update)
        .await
        .map_err(err)
}

/// 把一个本地文件拖拽导入实例:按 target 拷贝到对应子目录,返回落盘文件名。
/// target = "mod" / "resourcepack" / "shader" / "datapack"。
#[tauri::command]
#[specta::specta]
pub fn import_local_resource(
    root: String,
    id: String,
    target: String,
    path: String,
    world: Option<String>,
) -> CmdResult<String> {
    use mc_core::instance::PackKind;
    let inst = instance_of(&root, &id);
    let src = std::path::Path::new(&path);
    let w = world.as_deref();
    match target.as_str() {
        "mod" => mc_core::instance::mods::import_local_mod(&inst, src).map_err(err),
        "resourcepack" => {
            mc_core::instance::packs::import_local_pack(&inst, PackKind::ResourcePack, src, None).map_err(err)
        }
        "shader" => {
            mc_core::instance::packs::import_local_pack(&inst, PackKind::Shader, src, None).map_err(err)
        }
        "datapack" => {
            mc_core::instance::packs::import_local_pack(&inst, PackKind::Datapack, src, w).map_err(err)
        }
        other => Err(format!("不支持的导入目标: {other}")),
    }
}

/// 列出某实例下指定类型的包(资源包 / 光影 / 数据包),含启停态。
#[tauri::command]
#[specta::specta]
pub fn instance_packs(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    world: Option<String>,
) -> CmdResult<Vec<mc_core::instance::PackInfo>> {
    let inst = instance_of(&root, &id);
    Ok(mc_core::instance::list_packs(&inst, kind, world.as_deref()))
}

/// 启用/停用一个包(改 `.zip` ↔ `.zip.disabled`)。
#[tauri::command]
#[specta::specta]
pub fn set_pack_enabled(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    file_name: String,
    enabled: bool,
    world: Option<String>,
) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::packs::set_pack_enabled(&inst, kind, &file_name, enabled, world.as_deref())
        .map_err(err)
}

/// 删除一个包(移入系统回收站,可找回)。
#[tauri::command]
#[specta::specta]
pub fn delete_pack(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    file_name: String,
    world: Option<String>,
) -> CmdResult<()> {
    let inst = instance_of(&root, &id);
    mc_core::instance::packs::delete_pack(&inst, kind, &file_name, world.as_deref()).map_err(err)
}

/// 安装一个包(资源包 / 光影 / 数据包)的最新兼容版本到实例对应目录。`provider` 缺省
/// `modrinth`。返回落盘文件名;CurseForge 作者禁分发的文件经 `blocked` 回传(file 为空),
/// 前端走手动下载流而非假装成功。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub async fn install_pack(
    root: String,
    id: String,
    kind: mc_core::instance::PackKind,
    project: String,
    mc_version: String,
    world: Option<String>,
    provider: Option<String>,
) -> CmdResult<VersionInstallReport> {
    use mc_core::instance::PackKind;
    let inst = instance_of(&root, &id);
    let dl = make_downloader()?;
    let w = world.as_deref();
    match parse_provider(provider.as_deref())? {
        mc_core::modplatform::ProviderId::Modrinth => {
            let api = ModrinthApi::new();
            mc_core::instance::install_pack(&api, &dl, &inst, kind, &project, &mc_version, w)
                .await
                .map(|file| VersionInstallReport { file, ..Default::default() })
                .map_err(err)
        }
        pid @ mc_core::modplatform::ProviderId::CurseForge => {
            let p = provider_or_err(&make_registry(), pid)?;
            let versions = p.list_versions(&project, Some(&mc_version), None).await.map_err(err)?;
            let v = versions
                .iter()
                .find(|v| v.game_versions.iter().any(|g| g == mc_version.as_str()))
                .or_else(|| versions.first())
                .ok_or_else(|| format!("项目 {project} 没有兼容 {mc_version} 的版本"))?;
            let Some(file) = v.primary_file() else {
                return Err(format!("版本 {} 没有可下载文件", v.id));
            };
            if file.url.is_empty() {
                let dir = match kind {
                    PackKind::ResourcePack => "resourcepacks",
                    PackKind::Shader => "shaderpacks",
                    PackKind::Datapack => "datapacks",
                };
                return Ok(VersionInstallReport {
                    blocked: vec![cf_blocked_dto(&project, &v.id, &file.filename, dir)],
                    ..Default::default()
                });
            }
            mc_core::instance::packs::install_pack_version(&inst, &dl, kind, v, w)
                .await
                .map(|file| VersionInstallReport { file, ..Default::default() })
                .map_err(err)
        }
    }
}

/// Discover 多选 facet 过滤(可选)。空字段即"不按该维度过滤"。仅 Modrinth 消费这些
/// (Modrinth 把 loader 放在 categories 维度、环境是 `client_side`/`server_side` facet);
/// `provider==curseforge` 时这些被忽略,只用顶层 `game_version` / `loader`。
#[derive(Debug, Default, serde::Deserialize, specta::Type)]
pub struct SearchFacetsArg {
    /// 多选内容分类(每个各成一个 AND 组)。
    #[serde(default)]
    pub categories: Vec<String>,
    /// 多选 loader(合成一个 OR 组)。
    #[serde(default)]
    pub loaders: Vec<String>,
    /// 多选游戏版本(合成一个 OR 组)。
    #[serde(default)]
    pub game_versions: Vec<String>,
    /// 运行环境:`"client"` / `"server"`(其余忽略)。
    #[serde(default)]
    pub environment: Option<String>,
    /// 仅开源项目(License facet)。
    #[serde(default)]
    pub open_source: Option<bool>,
}

/// 跨平台内容搜索:`provider` 缺省 `modrinth`(也可 `curseforge`,需配 CF key),`sort`
/// 缺省按相关度。`facets` 是 Discover 的多选 facet 过滤(仅 Modrinth 消费)。经 Provider
/// 注册表路由,统一返回 [`SearchHit`]。命令名保持 `modrinth_search` 以稳定绑定,但实际是
/// 泛平台搜索。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
#[specta::specta]
pub async fn modrinth_search(
    query: String,
    kind: String,
    game_version: Option<String>,
    loader: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
    provider: Option<String>,
    sort: Option<String>,
    facets: Option<SearchFacetsArg>,
) -> CmdResult<Vec<mc_core::modplatform::SearchHit>> {
    use mc_core::modplatform::{SearchQuery, SortMethod};
    let kind = match kind.as_str() {
        "modpack" => ResourceKind::Modpack,
        "shader" => ResourceKind::Shader,
        "resourcepack" => ResourceKind::ResourcePack,
        "datapack" => ResourceKind::Datapack,
        _ => ResourceKind::Mod,
    };
    let sort = match sort.as_deref() {
        Some("downloads") => SortMethod::Downloads,
        Some("newest") => SortMethod::Newest,
        Some("updated") => SortMethod::Updated,
        _ => SortMethod::Relevance,
    };
    let facets = facets.unwrap_or_default();
    let q = SearchQuery {
        text: query,
        kind,
        game_version: game_version.filter(|s| !s.is_empty()),
        loader: loader.filter(|s| !s.is_empty()),
        game_versions: facets.game_versions,
        loaders: facets.loaders,
        categories: facets.categories,
        environment: facets.environment.filter(|s| !s.is_empty()),
        open_source: facets.open_source,
        offset: offset.unwrap_or(0),
        limit: limit.unwrap_or(30),
        sort,
    };
    let pid = parse_provider(provider.as_deref())?;
    let p = provider_or_err(&make_registry(), pid)?;
    p.search(&q).await.map_err(err)
}

/// Modrinth 的 facet 分类法(内容分类 / loader / 游戏版本),供 Discover 渲染过滤面板。
/// 进程内缓存(见 [`ModrinthApi::content_facets`]),仅 Modrinth 提供;CurseForge 不走此处。
#[tauri::command]
#[specta::specta]
pub async fn content_facets() -> CmdResult<mc_core::modplatform::modrinth::FacetTagsDto> {
    ModrinthApi::new().content_facets().await.map_err(err)
}

