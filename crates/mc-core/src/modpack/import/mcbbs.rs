//! MCBBS(国内)/ HMCL-lineage 整合包导入器。
//!
//! zip,根 `mcbbs.packmeta`(或带 `addons` 的 `manifest.json`)+ `overrides/`。mod 不在包内,
//! 经 CurseForge 拉(`files[]` 是 CurseForge-shaped `{projectID,fileID}`)。
//!
//! 与来源的桥(经 [`crate::modpack::formats::mcbbs::McbbsPackMeta`]):
//! - 标记 `mcbbs.packmeta`(唯一命名,根级)**或** `manifest.json` 且内容**有** `addons`/`launchInfo`
//!   —— 与 CurseForge 同名 `manifest.json` 靠**内容**区分(见 [`is_mcbbs_manifest`])。故本 importer
//!   注册在 curseforge **之前**。
//! - `addons[id=="game"].version` → `mc_version`;其余 addon(forge/neoforge/fabric/quilt/…)→ loader。
//! - `launchInfo.{javaArgument,launchArgument}` → 实例 JVM / 游戏参数;`minMemory` → 推荐内存。
//! - `files[]` → [`ImportPlan::unresolved`](`target_dir="mods"`),`resolve()` 复用 CurseForge 解析。
//! - override 根 = `["overrides"]`。
//!
//! `resolve()` 联网,复用 [`super::curseforge`] 的同一条 CF 批量解析 + blocked 检测路径。

use std::path::Path;

use futures::future::BoxFuture;

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::modpack::formats::mcbbs::McbbsPackMeta;
use crate::modplatform::provider::ProviderRegistry;

use super::curseforge::{loader_kind_from_family, resolve_curseforge_refs};
use super::{
    shallowest_marker, ArchiveIndex, BlockedFile, DetectMatch, ImportPlan, ManagedPack,
    ModpackImporter, UnresolvedRef,
};

/// MCBBS 唯一命名标记 basename。
const MARK_PACKMETA: &str = "mcbbs.packmeta";
/// 与 CurseForge 同名的标记 basename(靠内容判别)。
const MARK_MANIFEST: &str = "manifest.json";
/// MCBBS `files[]` 落盘目录。
const TARGET_DIR: &str = "mods";
/// MCBBS override 根。
const OVERRIDES: &str = "overrides";

/// MCBBS(国内)整合包导入器。
pub struct McbbsImporter;

impl ModpackImporter for McbbsImporter {
    fn id(&self) -> &'static str {
        "mcbbs"
    }

    fn detect(&self, archive: &dyn ArchiveIndex) -> Option<DetectMatch> {
        // 1) 唯一命名标记 mcbbs.packmeta —— 最高优先(无需读内容)。
        if let Some(marker) = shallowest_marker(archive, MARK_PACKMETA) {
            return Some(DetectMatch::from_marker(self.id(), &marker));
        }
        // 2) 与 CurseForge 同名的 manifest.json —— 只在内容**有** addons/launchInfo 时命中。
        let marker = shallowest_marker(archive, MARK_MANIFEST)?;
        let bytes = archive.read_small(&marker)?;
        if !is_mcbbs_manifest(&bytes) {
            return None;
        }
        Some(DetectMatch::from_marker(self.id(), &marker))
    }

    fn plan(&self, staging: &Path, _m: &DetectMatch) -> Result<ImportPlan> {
        // 优先读 mcbbs.packmeta;无则读 manifest.json(带 addons 的那种)。
        let (marker, raw) = read_packmeta(staging)?;
        let meta: McbbsPackMeta = serde_json::from_str(&raw)
            .map_err(|e| CoreError::Parse { what: marker.into(), source: e })?;
        plan_from_packmeta(&meta)
    }

    fn resolve<'a>(
        &'a self,
        dl: &'a Downloader,
        registry: &'a ProviderRegistry,
        plan: &'a mut ImportPlan,
    ) -> BoxFuture<'a, Result<Vec<BlockedFile>>> {
        // MCBBS files[] 是 CurseForge-shaped:复用 curseforge 的批量解析 + blocked 检测。
        Box::pin(async move { resolve_curseforge_refs(dl, registry, plan).await })
    }
}

/// 纯内容判别:字节是否是「MCBBS manifest」——顶层**有** `addons` 或 `launchInfo`
/// (与 CurseForge 同名 `manifest.json` 的唯一区分点)。
///
/// 与 [`super::curseforge::is_curseforge_manifest`] 互补:对同一 `manifest.json` 至多一个为真。
pub(crate) fn is_mcbbs_manifest(bytes: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        return false;
    };
    let has_addons = value.get("addons").map(|v| !v.is_null()).unwrap_or(false);
    let has_launch_info = value.get("launchInfo").map(|v| !v.is_null()).unwrap_or(false);
    has_addons || has_launch_info
}

/// 从 staging 取 packmeta:优先 `mcbbs.packmeta`,其次 `manifest.json`。返回(标记名, 内容)。
fn read_packmeta(staging: &Path) -> Result<(&'static str, String)> {
    let packmeta = staging.join(MARK_PACKMETA);
    if packmeta.is_file() {
        let raw = std::fs::read_to_string(&packmeta).map_err(|e| CoreError::io(&packmeta, e))?;
        return Ok((MARK_PACKMETA, raw));
    }
    let manifest = staging.join(MARK_MANIFEST);
    let raw = std::fs::read_to_string(&manifest).map_err(|e| CoreError::io(&manifest, e))?;
    Ok((MARK_MANIFEST, raw))
}

/// 纯解析:从一份已反序列化的 [`McbbsPackMeta`] 产出 [`ImportPlan`](files 待 resolve)。
///
/// 抽成自由函数便于单测(直接喂 fixture packmeta 解析后的结构,无需落盘 staging)。
pub(crate) fn plan_from_packmeta(meta: &McbbsPackMeta) -> Result<ImportPlan> {
    let mc_version = meta
        .minecraft_version()
        .ok_or_else(|| CoreError::other("MCBBS 整合包缺少 addons[id==\"game\"](Minecraft 版本)"))?
        .to_string();

    let pack_name = meta
        .name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "MCBBS Pack".to_string());

    let mut plan = ImportPlan::new(pack_name, mc_version);
    plan.pack_version = meta.version.clone().filter(|s| !s.is_empty());

    // loader:第一个非 game 的、能映射到已知家族的 addon → (LoaderKind, 版本)。
    // optifine 在 addons 里可能出现,但它不是组件 loader(loader_kind_from_family 给 OptiFine,
    // 引擎对它仅装原版),故仍取第一个**可映射**的 addon。
    plan.loader = meta
        .loader_addons()
        .find_map(|a| loader_kind_from_family(&a.id.to_ascii_lowercase()).map(|k| (k, a.version.clone())));

    // launchInfo:启动参数 / Java 参数 / 内存。JavaPath / 命令不在此(不可自动信任)。
    if let Some(li) = &meta.launch_info {
        plan.extra_jvm_args = li.java_argument.clone();
        plan.extra_game_args = li.launch_argument.clone();
        if let Some(min_mem) = li.min_memory {
            plan.recommended_ram_mib = Some(min_mem as u64);
        }
    }

    plan.override_roots = vec![OVERRIDES.to_string()];
    plan.managed = Some(ManagedPack {
        platform: "mcbbs".to_string(),
        project_id: meta.name.clone().unwrap_or_default(),
        version_id: plan.pack_version.clone(),
    });

    // files[] 是 CurseForge-shaped(projectID/fileID):全部进 unresolved 待 resolve()。
    for f in &meta.files {
        plan.unresolved.push(UnresolvedRef {
            project_id: f.project_id.to_string(),
            file_id: f.file_id.to_string(),
            target_dir: TARGET_DIR.to_string(),
            required: f.required,
        });
    }

    Ok(plan)
}

