//! 装「核心」—— 原版或原版 + 一个 mod 加载器 —— 的统一入口。
//!
//! 「从零建实例」与「导入整合包」是同一件事的两面:都要先把核心装到磁盘,拿到实例
//! 应 `inheritsFrom` 的版本 id(见 docs/modules/instance-and-components.md §3)。这里把
//! 那段「按 loader 分派到对应安装器、无 loader 走原版」的编排集中一处,两条路径共用。

use tokio::sync::watch;

use mc_types::{LoaderKind, Progress};

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::paths::GamePaths;

use super::{install_fabric, install_forge, install_neoforge, install_quilt};

/// 装核心,返回实例应 `inheritsFrom` 的版本 id:
/// - `loader == None` → 原版,返回 `mc_version`(缺失才装)。
/// - Fabric/Quilt → 拉 loader meta profile,返回其版本 id。
/// - Forge/NeoForge → 跑官方 installer,返回它生成的版本 id。
/// - 其它(LiteLoader/OptiFine 暂无独立安装器)→ 仅装原版,返回 `mc_version`。
///
/// 各 loader 安装器内部都会「缺原版先装原版」,故原版只在无 loader 时显式触发。
pub async fn install_core(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    loader: Option<&(LoaderKind, String)>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    let manifest = crate::meta::fetch_manifest(dl).await?;
    let vanilla_entry = manifest
        .iter()
        .find(|m| m.id == mc_version)
        .ok_or_else(|| CoreError::other(format!("版本清单中找不到 Minecraft {mc_version}")))?;

    let core_id = match loader {
        None => {
            if !paths.version_json(mc_version).is_file() {
                crate::launch::install_version(dl, paths, vanilla_entry, progress).await?;
            }
            mc_version.to_string()
        }
        Some((LoaderKind::Fabric, _)) => {
            install_fabric(dl, paths, mc_version, vanilla_entry, progress).await?
        }
        Some((LoaderKind::Quilt, _)) => {
            install_quilt(dl, paths, mc_version, vanilla_entry, progress).await?
        }
        Some((LoaderKind::Forge, build)) => {
            install_forge(dl, paths, mc_version, build, vanilla_entry, None, progress).await?
        }
        Some((LoaderKind::NeoForge, neo_version)) => {
            install_neoforge(dl, paths, neo_version, vanilla_entry, None, progress).await?
        }
        Some((other, _)) => {
            tracing::warn!(loader = other.as_str(), "该 loader 暂无自动安装器,仅装原版");
            if !paths.version_json(mc_version).is_file() {
                crate::launch::install_version(dl, paths, vanilla_entry, progress).await?;
            }
            mc_version.to_string()
        }
    };
    Ok(core_id)
}
