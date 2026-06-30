//! Modrinth `.mrpack` 导入器:把 `modrinth.index.json` 解析成 [`ImportPlan`]。
//!
//! 这是从 `instance/lifecycle.rs::import_mrpack` 重构出的格式知识 —— 引擎管线(下原版 /
//! 装 loader / 下文件 / 铺 overrides)全部上移到 [`super::engine`],本文件只懂 mrpack schema:
//!
//! - 标记文件 `modrinth.index.json`(根级)→ [`ModpackImporter::detect`]。
//! - `dependencies.minecraft` → `mc_version`;`fabric-loader`/`quilt-loader`/`forge`/`neoforge`
//!   → [`ImportPlan::loader`]。
//! - `files[]`:`downloads[]` 多源直接进 [`PlannedFile::sources`];`hashes.sha512`(规范哈希)
//!   + 可选 `sha1`;`env.client == unsupported` 跳过、`== optional` 降级为可选。
//! - override 根 = `["overrides", "client-overrides"]`(后者覆盖前者)。
//!
//! mrpack 自带 URL,故**不**覆盖 `resolve()`(用 trait 默认空操作)。`plan()` 纯函数,可对
//! fixture 单测(见 [`super::tests`])。

use std::path::Path;

use mc_types::LoaderKind;

use crate::error::{CoreError, Result};
use crate::modpack::formats::mrpack::{EnvSupport, MrpackIndex, MRPACK_DOWNLOAD_HOSTS};

use super::{
    basename, depth, ArchiveIndex, DetectMatch, ImportPlan, ManagedPack, ModpackImporter,
    PlannedFile,
};

/// `modrinth.index.json` 的标记 basename。
const MARK: &str = "modrinth.index.json";
/// 通用 override 根(客户端 + 服务端)。
pub(crate) const OVERRIDES: &str = "overrides";
/// 客户端专属 override 根(盖在 `overrides` 上)。
pub(crate) const CLIENT_OVERRIDES: &str = "client-overrides";

/// URL 的 host 是否在 mrpack 下载白名单内([`MRPACK_DOWNLOAD_HOSTS`]):等于某项或为其子域。
/// 仅放行白名单 host;恶意包指向任意 host 的源会被滤掉(纵深防御)。解析与后缀匹配都委托给
/// [`crate::host`](按字节比较,与重构前一致 —— 导入侧不做大小写归一)。
fn host_allowed(url: &str) -> bool {
    match crate::host::host_of(url) {
        Some(host) => crate::host::host_matches_suffix(host, MRPACK_DOWNLOAD_HOSTS),
        None => false,
    }
}

/// Modrinth `.mrpack` 导入器。
pub struct ModrinthImporter;

impl ModpackImporter for ModrinthImporter {
    fn id(&self) -> &'static str {
        "modrinth"
    }

    fn detect(&self, archive: &dyn ArchiveIndex) -> Option<DetectMatch> {
        // 找 basename 恰为 modrinth.index.json 的最浅条目(嵌套包根 / 防 overrides 内误判)。
        let marker = archive
            .entries()
            .iter()
            .filter(|e| basename(e) == MARK)
            .min_by_key(|e| depth(e))?;
        Some(DetectMatch::from_marker(self.id(), marker))
    }

    fn plan(&self, staging: &Path, _m: &DetectMatch) -> Result<ImportPlan> {
        let raw = std::fs::read_to_string(staging.join(MARK))
            .map_err(|e| CoreError::io(staging.join(MARK), e))?;
        let index: MrpackIndex = serde_json::from_str(&raw)
            .map_err(|e| CoreError::Parse { what: MARK.into(), source: e })?;
        plan_from_index(&index)
    }
}

/// 纯解析:从一份已反序列化的 [`MrpackIndex`] 产出 [`ImportPlan`]。
///
/// 抽出来便于单测(直接喂 fixture json 解析后的结构,无需落盘 staging)。
pub(crate) fn plan_from_index(index: &MrpackIndex) -> Result<ImportPlan> {
    // formatVersion 仅 v1 规范;未知版本按 v1 语义继续(字段语义至今无破坏性变更)。
    if index.format_version != 1 {
        tracing::warn!(
            format_version = index.format_version,
            "未知的 mrpack formatVersion,按 v1 语义继续导入"
        );
    }

    let mc_version = index
        .dependencies
        .minecraft
        .clone()
        .ok_or_else(|| CoreError::other(".mrpack dependencies 缺少 minecraft 版本"))?;

    let mut plan = ImportPlan::new(
        if index.name.is_empty() { "Modrinth Pack".to_string() } else { index.name.clone() },
        mc_version,
    );
    plan.pack_version = (!index.version_id.is_empty()).then(|| index.version_id.clone());
    plan.loader = loader_from_dependencies(&index.dependencies);
    plan.override_roots = vec![OVERRIDES.to_string(), CLIENT_OVERRIDES.to_string()];
    plan.managed = Some(ManagedPack {
        platform: "modrinth".to_string(),
        project_id: index.name.clone(),
        version_id: plan.pack_version.clone(),
    });

    for f in &index.files {
        // 纯服务端文件(client unsupported)在客户端导入时跳过。
        if !f.client_supported() {
            continue;
        }
        // 只保留白名单 host 的下载源,过滤掉指向任意 host 的恶意源(纵深防御)。
        let sources: Vec<String> =
            f.downloads.iter().filter(|u| host_allowed(u)).cloned().collect();
        // 无任何(合法)下载源的条目无法处理(overrides 已覆盖这类本地文件),跳过。
        if sources.is_empty() {
            continue;
        }
        let required = !matches!(f.env.map(|e| e.client), Some(EnvSupport::Optional));
        plan.files.push(PlannedFile {
            rel_path: f.path.clone(),
            sources,
            sha1: f.hashes.sha1.clone(),
            sha512: Some(f.hashes.sha512.clone()).filter(|s| !s.is_empty()),
            size: f.file_size,
            required,
        });
    }

    Ok(plan)
}

/// 把 mrpack `dependencies` 里的 loader 键映射成 `(LoaderKind, 版本)`。
///
/// 规范保证至多一种 loader(`deny_unknown_fields` 已挡未知键)。优先级按家族枚举固定顺序
/// 取第一个出现的(实际不会同时出现多种)。
fn loader_from_dependencies(
    deps: &crate::modpack::formats::mrpack::MrpackDependencies,
) -> Option<(LoaderKind, String)> {
    if let Some(v) = &deps.neoforge {
        return Some((LoaderKind::NeoForge, v.clone()));
    }
    if let Some(v) = &deps.forge {
        return Some((LoaderKind::Forge, v.clone()));
    }
    if let Some(v) = &deps.fabric_loader {
        return Some((LoaderKind::Fabric, v.clone()));
    }
    if let Some(v) = &deps.quilt_loader {
        return Some((LoaderKind::Quilt, v.clone()));
    }
    None
}

#[cfg(test)]
mod host_tests {
    use super::*;
    use crate::modpack::formats::mrpack::{MrpackDependencies, MrpackFile, MrpackHashes};

    #[test]
    fn allowlisted_hosts_pass() {
        assert!(host_allowed("https://cdn.modrinth.com/data/x/sodium.jar"));
        assert!(host_allowed("https://raw.githubusercontent.com/o/r/main/a.jar"));
        assert!(host_allowed("https://github.com/o/r/releases/a.jar"));
        assert!(host_allowed("https://gitlab.com/o/r/-/raw/main/a.jar"));
        // 子域亦放行(== allowed 或 ends_with(".allowed"))。
        assert!(host_allowed("https://media.forge.cdn.modrinth.com/x.jar"));
    }

    #[test]
    fn random_host_is_filtered() {
        assert!(!host_allowed("https://evil.example.com/x.jar"));
        // 不能被「以白名单结尾的恶意域」绕过。
        assert!(!host_allowed("https://github.com.evil.com/x.jar"));
        assert!(!host_allowed("https://notgithub.com/x.jar"));
        // 缺 scheme 一律拒绝。
        assert!(!host_allowed("cdn.modrinth.com/x.jar"));
        // userinfo / port 不影响 host 判定。
        assert!(host_allowed("https://user@cdn.modrinth.com:443/x.jar"));
    }

    fn idx_with_downloads(downloads: Vec<&str>) -> MrpackIndex {
        MrpackIndex {
            format_version: 1,
            game: "minecraft".to_string(),
            version_id: "1.0.0".to_string(),
            name: "Pack".to_string(),
            summary: None,
            dependencies: MrpackDependencies {
                minecraft: Some("1.20.1".to_string()),
                ..Default::default()
            },
            files: vec![MrpackFile {
                path: "mods/a.jar".to_string(),
                hashes: MrpackHashes { sha512: "h".to_string(), sha1: None },
                env: None,
                downloads: downloads.into_iter().map(String::from).collect(),
                file_size: None,
            }],
        }
    }

    #[test]
    fn disallowed_sources_are_dropped() {
        // 混合源:仅保留白名单 host。
        let plan = plan_from_index(&idx_with_downloads(vec![
            "https://evil.example.com/a.jar",
            "https://cdn.modrinth.com/a.jar",
        ]))
        .unwrap();
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].sources, vec!["https://cdn.modrinth.com/a.jar".to_string()]);
    }

    #[test]
    fn file_with_only_disallowed_source_is_skipped() {
        // 唯一源不在白名单 → 文件被空源守卫丢弃。
        let plan = plan_from_index(&idx_with_downloads(vec!["https://evil.example.com/a.jar"]))
            .unwrap();
        assert!(plan.files.is_empty());
    }
}
