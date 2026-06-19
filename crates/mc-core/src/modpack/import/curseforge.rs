//! CurseForge 整合包导入器:把 `manifest.json`(`manifestType=="minecraftModpack"`)解析成
//! [`ImportPlan`],并覆盖 [`ModpackImporter::resolve`] 把 `projectID`/`fileID` 经 Flame API
//! 变成真实下载 URL。
//!
//! 这是从格式知识([`crate::modpack::formats::curseforge::FlameManifest`])到统一 plan 的
//! 唯一桥:
//!
//! - 标记文件 `manifest.json`(根级)且内容**无** `addons`/`launchInfo` → CurseForge
//!   (有则归 MCBBS,见 [`super::mcbbs`])。`manifest.json` 会出现在 `overrides/` 里,故只取**根最浅**
//!   的命中并要求内容判别通过(`detect()` 内读 `read_small`)。
//! - `minecraft.version` → `mc_version`;`modLoaders[].id` 经 `split_id()` 前缀解析家族 + 版本。
//! - `files[]` 只给 `projectID`/`fileID` → 全部进 [`ImportPlan::unresolved`](`target_dir="mods"`),
//!   `required` 直接取 manifest(默认 true)。
//! - override 根 = `[manifest.overrides]`(默认 `"overrides"`)。
//!
//! `resolve()`(联网)经 `registry` 里的 CurseForge [`ResourceProvider`] 批量查文件,把解析出的
//! url(+ 可选镜像)写进 [`PlannedFile::sources`];`download_url` 为空(BLOCKED,作者禁第三方
//! 分发)的文件**绝不**猜 URL,收集成 [`BlockedFile`] 回传给 UI 手动下载。

use std::path::Path;

use futures::future::BoxFuture;

use mc_types::LoaderKind;

use crate::download::{Downloader, MirrorResolver};
use crate::error::{CoreError, Result};
use crate::modpack::formats::curseforge::FlameManifest;
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::ProviderId;

use super::{
    ArchiveIndex, BlockedFile, DetectMatch, ImportPlan, ManagedPack, ModpackImporter, PlannedFile,
    UnresolvedRef,
};

/// CurseForge manifest 的标记 basename。
const MARK: &str = "manifest.json";
/// CurseForge `files[]` 落盘目录(整合包文件都是 mod)。
const TARGET_DIR: &str = "mods";

/// CurseForge `manifest.json` 导入器。
pub struct CurseForgeImporter;

impl ModpackImporter for CurseForgeImporter {
    fn id(&self) -> &'static str {
        "curseforge"
    }

    fn detect(&self, archive: &dyn ArchiveIndex) -> Option<DetectMatch> {
        // 取 basename 恰为 manifest.json 的最浅命中(防 overrides 内同名文件误判)。
        let marker = archive
            .entries()
            .iter()
            .filter(|e| basename(e) == MARK)
            .min_by_key(|e| depth(e))?;
        // 内容判别:必须是 CF modpack manifest 且**无** addons/launchInfo(有则归 MCBBS)。
        let bytes = archive.read_small(marker)?;
        if !is_curseforge_manifest(&bytes) {
            return None;
        }
        Some(DetectMatch::from_marker(self.id(), marker))
    }

    fn plan(&self, staging: &Path, _m: &DetectMatch) -> Result<ImportPlan> {
        let raw = std::fs::read_to_string(staging.join(MARK))
            .map_err(|e| CoreError::io(staging.join(MARK), e))?;
        let manifest: FlameManifest = serde_json::from_str(&raw)
            .map_err(|e| CoreError::Parse { what: MARK.into(), source: e })?;
        plan_from_manifest(&manifest)
    }

    fn resolve<'a>(
        &'a self,
        dl: &'a Downloader,
        registry: &'a ProviderRegistry,
        plan: &'a mut ImportPlan,
    ) -> BoxFuture<'a, Result<Vec<BlockedFile>>> {
        Box::pin(async move { resolve_curseforge_refs(dl, registry, plan).await })
    }
}

/// 纯内容判别:字节是否是「CurseForge 整合包 manifest」——`manifestType=="minecraftModpack"`
/// 且顶层**无** `addons` / `launchInfo`(有任一即 MCBBS,归 [`super::mcbbs`])。
///
/// 抽成自由函数便于单测(detect 的核心判别),且与 [`super::mcbbs::is_mcbbs_manifest`] 互补:
/// 二者对同一 `manifest.json` 至多一个返回 true。
pub(crate) fn is_curseforge_manifest(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    let manifest_type = value
        .get("manifestType")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if manifest_type != "minecraftModpack" {
        return false;
    }
    let has_addons = value.get("addons").map(|v| !v.is_null()).unwrap_or(false);
    let has_launch_info = value.get("launchInfo").map(|v| !v.is_null()).unwrap_or(false);
    !has_addons && !has_launch_info
}

/// 纯解析:从一份已反序列化的 [`FlameManifest`] 产出 [`ImportPlan`](不含 sources,待 resolve)。
///
/// 抽出来便于单测(直接喂 fixture manifest 解析后的结构,无需落盘 staging)。
pub(crate) fn plan_from_manifest(manifest: &FlameManifest) -> Result<ImportPlan> {
    // manifestType/Version 不符直接拒(不做尽力解析,会错配 id)。
    if !manifest.is_valid() {
        return Err(CoreError::other(format!(
            "不是有效的 CurseForge 整合包 manifest(manifestType={:?} manifestVersion={})",
            manifest.manifest_type, manifest.manifest_version
        )));
    }

    let mc_version = manifest.minecraft.version.clone();
    if mc_version.is_empty() {
        return Err(CoreError::other("CurseForge manifest 缺少 minecraft.version"));
    }

    let mut plan = ImportPlan::new(
        if manifest.name.is_empty() { "CurseForge Pack".to_string() } else { manifest.name.clone() },
        mc_version,
    );
    plan.pack_version = (!manifest.version.is_empty()).then(|| manifest.version.clone());
    plan.loader = loader_from_manifest(manifest);
    // recommendedRam 是 MB;<=0 视为未给。
    if manifest.minecraft.recommended_ram > 0 {
        plan.recommended_ram_mib = Some(manifest.minecraft.recommended_ram as u64);
    }
    // override 根:manifest.overrides(默认 "overrides")。
    plan.override_roots = vec![manifest.overrides.clone()];
    plan.managed = Some(ManagedPack {
        platform: "curseforge".to_string(),
        project_id: manifest.name.clone(),
        version_id: plan.pack_version.clone(),
    });

    // files[] 只给 id,全部进 unresolved 待 resolve() 查 URL。
    for f in &manifest.files {
        plan.unresolved.push(UnresolvedRef {
            project_id: f.project_id.to_string(),
            file_id: f.file_id.to_string(),
            target_dir: TARGET_DIR.to_string(),
            required: f.required,
        });
    }

    Ok(plan)
}

/// 把 CurseForge 的 primary loader id 映射成 `(LoaderKind, 版本)`。无 loader → `None`(原版)。
fn loader_from_manifest(manifest: &FlameManifest) -> Option<(LoaderKind, String)> {
    let loader = manifest.minecraft.primary_loader()?;
    let (family, version) = loader.split_id();
    loader_kind_from_family(&family).map(|kind| (kind, version))
}

/// 把 loader 家族字符串(小写)映射到 [`LoaderKind`]。CF / MCBBS 共用。
///
/// 未知家族 → `None`(按原版处理,而非误装错 loader)。
pub(crate) fn loader_kind_from_family(family: &str) -> Option<LoaderKind> {
    match family {
        "forge" => Some(LoaderKind::Forge),
        "neoforge" => Some(LoaderKind::NeoForge),
        "fabric" => Some(LoaderKind::Fabric),
        "quilt" => Some(LoaderKind::Quilt),
        "liteloader" => Some(LoaderKind::LiteLoader),
        "optifine" => Some(LoaderKind::OptiFine),
        _ => None,
    }
}

/// `resolve()` 的实现:经 CurseForge provider 把 `plan.unresolved` 的 (projectID,fileID) 批量
/// 查成具体下载源,填进 `plan.files`;`download_url` 为空(BLOCKED)的收集成 [`BlockedFile`]。
///
/// `pub(crate)`:CurseForge 与 MCBBS(`files[]` 同为 CurseForge-shaped)共用同一条解析路径。
pub(crate) async fn resolve_curseforge_refs(
    _dl: &Downloader,
    registry: &ProviderRegistry,
    plan: &mut ImportPlan,
) -> Result<Vec<BlockedFile>> {
    if plan.unresolved.is_empty() {
        return Ok(Vec::new());
    }

    let provider = registry.get(ProviderId::CurseForge).ok_or_else(|| {
        CoreError::other(
            "导入 CurseForge 整合包需配置 CurseForge API key(环境变量 MC_CF_API_KEY)",
        )
    })?;

    // 批量按 (project_id, file_id) 取文件(顺序不保证与输入一致 → 用 file_id 对齐)。
    let refs: Vec<(String, String)> = plan
        .unresolved
        .iter()
        .map(|u| (u.project_id.clone(), u.file_id.clone()))
        .collect();
    let resolved = provider.get_files_bulk(&refs).await?;

    use std::collections::HashMap;
    // version_id == CurseForge fileId(见 curseforge::resolved_from_file);据此对齐回 unresolved。
    let by_file_id: HashMap<&str, &crate::modplatform::ResolvedFile> =
        resolved.iter().map(|r| (r.version_id.as_str(), r)).collect();

    let mut blocked: Vec<BlockedFile> = Vec::new();
    // 取走 unresolved(已逐项处理),避免引擎二次解析。
    let unresolved = std::mem::take(&mut plan.unresolved);
    for u in unresolved {
        let Some(file) = by_file_id.get(u.file_id.as_str()) else {
            // 平台查不到该 file:必备则报错,可选则当作 blocked 让用户处理。
            if u.required {
                return Err(CoreError::other(format!(
                    "CurseForge 文件未找到(projectID={} fileID={})",
                    u.project_id, u.file_id
                )));
            }
            continue;
        };

        let filename = sanitize_filename(&file.file.filename);
        if filename.is_empty() {
            // 没有文件名无法落盘:视为不可处理(必备则报错)。
            if u.required {
                return Err(CoreError::other(format!(
                    "CurseForge 文件缺少文件名(fileID={})",
                    u.file_id
                )));
            }
            continue;
        }

        // download_url 为空 = BLOCKED(作者禁第三方分发):绝不猜 URL,走手动下载流。
        if file.file.url.is_empty() {
            blocked.push(BlockedFile {
                name: filename,
                website_url: blocked_website_url(&u.project_id, &u.file_id),
                target_dir: u.target_dir.clone(),
                required: u.required,
            });
            continue;
        }

        let rel_path = format!("{}/{}", u.target_dir.trim_end_matches('/'), filename);
        // 主源 = 平台给的 forgecdn url;追加镜像源(BMCLAPI 等)做多源故障转移。
        let sources = source_candidates(&file.file.url);
        plan.files.push(PlannedFile {
            rel_path,
            sources,
            sha1: file.file.sha1.clone(),
            sha512: file.file.sha512.clone(),
            size: file.file.size,
            required: u.required,
        });
    }

    Ok(blocked)
}

/// 由 (projectID, fileID) 拼出 BLOCKED 文件的官网手动下载页 URL(供 UI 引导用户)。
fn blocked_website_url(project_id: &str, file_id: &str) -> String {
    // CurseForge 的稳定下载入口:`/api/v1/mods/{projectID}/files/{fileID}/download`
    // 不可用于第三方自动下载,但作为给用户的"去这里手动下"链接是规范做法。
    format!("https://www.curseforge.com/api/v1/mods/{project_id}/files/{file_id}/download")
}

/// 把平台给的主源扩成有序候选源:主源在前,再追加可用镜像(BMCLAPI 对 forgecdn 的重写)。
fn source_candidates(primary: &str) -> Vec<String> {
    let resolver = MirrorResolver::china();
    let mut out = vec![primary.to_string()];
    for mirror in resolver.candidates(primary) {
        if mirror != primary && !out.contains(&mirror) {
            out.push(mirror);
        }
    }
    out
}

/// 清洗 CurseForge 文件名:去掉路径分隔符与非法字符,**绝不**允许 `..`/分隔符进 rel_path
/// (写盘前的第一道闸;引擎随后还会过 safe_join)。
fn sanitize_filename(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .replace('\0', "")
}

/// 条目 basename(最后一个 `/` 之后)。
fn basename(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

/// 路径深度(`/` 段数);用于在多命中里取最浅根。
fn depth(entry: &str) -> usize {
    entry.split('/').filter(|s| !s.is_empty()).count()
}
