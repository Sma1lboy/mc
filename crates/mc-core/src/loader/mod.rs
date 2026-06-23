//! Mod-loader installation. A loader install boils down to "obtain the loader's
//! version json (which `inheritsFrom` the vanilla version) and drop it on disk";
//! after that it is just another component the version system merges. See
//! `docs/modules/version-system.md`.

pub mod core;
pub mod fabric;
pub mod forge;
pub mod installer;
pub mod neoforge;
pub mod quilt;
pub mod versions;

pub use core::install_core;
pub use fabric::install_fabric;
pub use forge::install_forge;
pub use neoforge::install_neoforge;
pub use quilt::install_quilt;
pub use versions::list_loader_versions;

use mc_types::LoaderKind;

/// 从实例的版本 id(== `versions/<id>` 目录名)里提取**干净的 loader 构建号**。
///
/// 实例对外暴露的 `loader_version` 只是整段版本 id(如 `fabric-loader-0.15.7-1.20.1`
/// 或 `1.20.1-forge-47.2.0`)。把它原样当作 loader 依赖值写进整合包,会让其它启动器
/// (以及本启动器自己)**再导入**时匹配不到 loader —— mrpack / CurseForge 规范要的是裸
/// 构建号(`0.15.7` / `47.2.0`)。本函数按各 loader 的命名约定解析:
///
/// - Fabric:`fabric-loader-<loaderver>-<mcver>` → `<loaderver>`
/// - Quilt :`quilt-loader-<loaderver>-<mcver>`  → `<loaderver>`
/// - Forge :`<mcver>-forge-<build>`              → `<build>`
/// - NeoForge:`neoforge-<ver>` / `<mcver>-neoforge-<ver>` → `<ver>`
///
/// 任何不符合预期形态的输入都**原样返回**(绝不比现状更糟)。
pub fn clean_loader_version(version_id: &str, kind: LoaderKind, mc_version: &str) -> String {
    let id = version_id.trim();
    let cleaned = match kind {
        LoaderKind::Fabric => strip_meta_loader_id(id, "fabric-loader-", mc_version),
        LoaderKind::Quilt => strip_meta_loader_id(id, "quilt-loader-", mc_version),
        LoaderKind::Forge => id.split_once("-forge-").map(|(_, v)| v.to_string()),
        LoaderKind::NeoForge => id
            .strip_prefix("neoforge-")
            .map(str::to_string)
            .or_else(|| id.split_once("-neoforge-").map(|(_, v)| v.to_string())),
        _ => None,
    };
    cleaned
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| id.to_string())
}

/// Fabric/Quilt 的 meta id 形如 `<prefix><loaderver>-<mcver>`:剥前缀,再剥 `-<mcver>` 后缀。
/// 缺前缀返回 `None`(交由上层兜底);缺 mc 后缀时退回剥前缀后的剩余部分。
fn strip_meta_loader_id(id: &str, prefix: &str, mc_version: &str) -> Option<String> {
    let rest = id.strip_prefix(prefix)?;
    let lv = rest.strip_suffix(&format!("-{mc_version}")).unwrap_or(rest);
    (!lv.is_empty()).then(|| lv.to_string())
}

#[cfg(test)]
mod loader_version_tests {
    use super::*;

    #[test]
    fn fabric_and_quilt_strip_prefix_and_mc_suffix() {
        assert_eq!(clean_loader_version("fabric-loader-0.15.7-1.20.1", LoaderKind::Fabric, "1.20.1"), "0.15.7");
        assert_eq!(clean_loader_version("quilt-loader-0.26.0-1.21", LoaderKind::Quilt, "1.21"), "0.26.0");
    }

    #[test]
    fn forge_takes_build_after_marker() {
        assert_eq!(clean_loader_version("1.20.1-forge-47.2.0", LoaderKind::Forge, "1.20.1"), "47.2.0");
        assert_eq!(clean_loader_version("1.12.2-forge-14.23.5.2859", LoaderKind::Forge, "1.12.2"), "14.23.5.2859");
    }

    #[test]
    fn neoforge_handles_both_id_shapes() {
        assert_eq!(clean_loader_version("neoforge-20.4.237", LoaderKind::NeoForge, "1.20.4"), "20.4.237");
        assert_eq!(clean_loader_version("1.20.4-neoforge-20.4.237", LoaderKind::NeoForge, "1.20.4"), "20.4.237");
    }

    #[test]
    fn unrecognized_shapes_pass_through_unchanged() {
        // 没有 -forge- 标记 → 原样返回,不会写出一个更坏的值。
        assert_eq!(clean_loader_version("weird-id", LoaderKind::Forge, "1.20.1"), "weird-id");
        // Fabric id 缺 mc 后缀 → 退回剥前缀后的剩余。
        assert_eq!(clean_loader_version("fabric-loader-0.15.7", LoaderKind::Fabric, "1.20.1"), "0.15.7");
    }
}
