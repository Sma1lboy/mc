//! 整合包 / 实例格式的字段级 serde 结构 + 格式探测。
//!
//! 子模块 [`formats`] 提供每种格式的精确结构(`modrinth.index.json` / `manifest.json` /
//! `mmc-pack.json` + `instance.cfg` / `mcbbs.packmeta` / packwiz TOML / ATLauncher /
//! Technic),可直接落成 importer `plan()` 的输入。本模块本身只提供:
//! - [`ModpackFormat`]:可被本启动器识别 / 导入的本地格式枚举。
//! - [`detect_format`]:对一个 zip / 目录的条目清单按**优先级判别表**(见
//!   `docs/modules/modpack-formats.md` §0)分派出格式 + 命中的标记条目路径。
//!
//! 探测只看条目清单 + 必要时按需读 manifest 内容(CF vs MCBBS 同名 `manifest.json`
//! 靠内容区分),**不**解压、**不**网络;真正的解析 / 安装由 importer 引擎驱动。

pub mod export;
pub mod formats;
pub mod import;

use serde::{Deserialize, Serialize};

/// 本启动器可识别 / 导入的本地整合包 / 实例格式。
///
/// 注:`ATLauncher` 是纯远程(无本地标记文件,用户从平台选包),故不在本地探测的产物里;
/// 远程包由 importer 的 RemotePackProvider 处理,不经 [`detect_format`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModpackFormat {
    Modrinth,
    CurseForge,
    MultiMc,
    Mcbbs,
    Packwiz,
    Technic,
}

/// 各格式的标记文件 basename(与判别表对应)。
const MARK_MCBBS: &str = "mcbbs.packmeta";
const MARK_MMC: &str = "mmc-pack.json";
const MARK_INSTANCE_CFG: &str = "instance.cfg";
const MARK_MODRINTH: &str = "modrinth.index.json";
const MARK_MANIFEST: &str = "manifest.json";
const MARK_PACKWIZ: &str = "pack.toml";
const MARK_TECHNIC_JAR: &str = "bin/modpack.jar";
const MARK_TECHNIC_VERSION: &str = "bin/version.json";

/// 探测一组归档 / 目录条目的整合包格式。
///
/// `entries` 是 zip / 目录里所有**文件**条目的相对路径(用 `/` 作分隔符;目录条目可含可不含)。
/// `read_manifest` 是按需取某条目字节的回调(仅 CF vs MCBBS 同名 `manifest.json` 判别时才调,
/// 避免无谓解压);返回 `None` 表示该条目读不到(则保守按 CurseForge 处理)。
///
/// 返回 `Some((格式, 命中标记条目的相对路径))`,标记路径携带**嵌套根**信息:整合包常被多包
/// 一层目录(如 `MyPack/modrinth.index.json`),命中路径的父目录即真实包根,供 importer 定位
/// `overrides/` 等。无任何已知标记则 `None`(调用方据此报「无法识别 / 请重压为 zip」)。
///
/// 判别优先级(`docs/modules/modpack-formats.md` §0):
/// 1. `mcbbs.packmeta`         → [`ModpackFormat::Mcbbs`]
/// 2. `mmc-pack.json` / `instance.cfg` → [`ModpackFormat::MultiMc`](目录即包,可嵌套一层)
/// 3. `modrinth.index.json`    → [`ModpackFormat::Modrinth`]
/// 4. `manifest.json`          → 读内容:有 `addons`/`launchInfo` 则 [`ModpackFormat::Mcbbs`],否则 [`ModpackFormat::CurseForge`]
/// 5. `pack.toml`              → [`ModpackFormat::Packwiz`]
/// 6. `bin/modpack.jar` / `bin/version.json` → [`ModpackFormat::Technic`]
///
/// 同一优先级内取**根最浅**的命中(`depth` 最小),保证嵌套包不被内层同名文件误判。
pub fn detect_format(
    entries: &[String],
    read_manifest: impl Fn(&str) -> Option<Vec<u8>>,
) -> Option<(ModpackFormat, String)> {
    // 归一:统一分隔符、去掉前导 `./`,只保留非空文件路径。
    let norm: Vec<String> = entries
        .iter()
        .map(|e| e.replace('\\', "/"))
        .map(|e| e.trim_start_matches("./").to_string())
        .filter(|e| !e.is_empty() && !e.ends_with('/'))
        .collect();

    // 1) mcbbs.packmeta —— 在 manifest.json 之前。
    if let Some(path) = find_by_basename(&norm, MARK_MCBBS) {
        return Some((ModpackFormat::Mcbbs, path));
    }

    // 2) MultiMC/Prism —— basename mmc-pack.json 或 instance.cfg(目录即包,可嵌套一层)。
    //    取两者中根更浅的命中作为标记。
    let mmc = find_by_basename(&norm, MARK_MMC);
    let cfg = find_by_basename(&norm, MARK_INSTANCE_CFG);
    if let Some(path) = shallowest(mmc, cfg) {
        return Some((ModpackFormat::MultiMc, path));
    }

    // 3) Modrinth —— modrinth.index.json(根级)。
    if let Some(path) = find_by_basename(&norm, MARK_MODRINTH) {
        return Some((ModpackFormat::Modrinth, path));
    }

    // 4 / 4b) manifest.json —— 读内容区分 CurseForge(无 addons)与 MCBBS(有 addons/launchInfo)。
    if let Some(path) = find_by_basename(&norm, MARK_MANIFEST) {
        let format = classify_manifest(&path, &read_manifest);
        return Some((format, path));
    }

    // 5) packwiz —— pack.toml。
    if let Some(path) = find_by_basename(&norm, MARK_PACKWIZ) {
        return Some((ModpackFormat::Packwiz, path));
    }

    // 6) Technic —— bin/modpack.jar / bin/version.json(解压到 minecraft/)。
    let tech_jar = find_by_suffix(&norm, MARK_TECHNIC_JAR);
    let tech_ver = find_by_suffix(&norm, MARK_TECHNIC_VERSION);
    if let Some(path) = shallowest(tech_jar, tech_ver) {
        return Some((ModpackFormat::Technic, path));
    }

    None
}

/// 读 `manifest.json` 内容判别 CF vs MCBBS:有 `addons` 或 `launchInfo` 顶层键 → MCBBS,
/// 否则 → CurseForge。读不到 / 解析失败 → 保守按 CurseForge(它是 `manifest.json` 的默认归属)。
fn classify_manifest(path: &str, read_manifest: &impl Fn(&str) -> Option<Vec<u8>>) -> ModpackFormat {
    let Some(bytes) = read_manifest(path) else {
        return ModpackFormat::CurseForge;
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return ModpackFormat::CurseForge;
    };
    let has_addons = value.get("addons").map(|v| !v.is_null()).unwrap_or(false);
    let has_launch_info = value
        .get("launchInfo")
        .map(|v| !v.is_null())
        .unwrap_or(false);
    if has_addons || has_launch_info {
        ModpackFormat::Mcbbs
    } else {
        ModpackFormat::CurseForge
    }
}

/// 找 basename 恰为 `name` 的条目;多个命中取根最浅(`/` 段数最少)的。
fn find_by_basename(entries: &[String], name: &str) -> Option<String> {
    entries
        .iter()
        .filter(|e| basename(e) == name)
        .min_by_key(|e| depth(e))
        .cloned()
}

/// 找以 `suffix`(可含 `/`,如 `bin/modpack.jar`)结尾的条目;多个取根最浅。
fn find_by_suffix(entries: &[String], suffix: &str) -> Option<String> {
    entries
        .iter()
        .filter(|e| *e == suffix || e.ends_with(&format!("/{suffix}")))
        .min_by_key(|e| depth(e))
        .cloned()
}

/// 取条目的 basename(最后一个 `/` 之后)。
fn basename(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

/// 路径深度(`/` 段数);用于在同类命中中取最浅根。
fn depth(entry: &str) -> usize {
    entry.split('/').filter(|s| !s.is_empty()).count()
}

/// 在两个可选命中中取根更浅的那个(都为 `None` 则 `None`)。
fn shallowest(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(x), Some(y)) => {
            if depth(&x) <= depth(&y) {
                Some(x)
            } else {
                Some(y)
            }
        }
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 无 manifest 内容的回调(用于不需要读内容的用例)。
    fn no_read(_: &str) -> Option<Vec<u8>> {
        None
    }

    fn entries(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn detects_modrinth_at_root() {
        let e = entries(&["modrinth.index.json", "overrides/mods/a.jar"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Modrinth);
        assert_eq!(path, "modrinth.index.json");
    }

    #[test]
    fn detects_modrinth_nested_one_level() {
        // 嵌套根:命中路径携带父目录,供 importer 定位 overrides。
        let e = entries(&["MyPack/modrinth.index.json", "MyPack/overrides/mods/a.jar"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Modrinth);
        assert_eq!(path, "MyPack/modrinth.index.json");
    }

    #[test]
    fn mcbbs_packmeta_beats_manifest() {
        // mcbbs.packmeta 优先级高于 manifest.json。
        let e = entries(&["mcbbs.packmeta", "manifest.json", "overrides/x"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Mcbbs);
        assert_eq!(path, "mcbbs.packmeta");
    }

    #[test]
    fn manifest_without_addons_is_curseforge() {
        let e = entries(&["manifest.json", "overrides/config/a.cfg"]);
        let body = br#"{ "manifestType": "minecraftModpack", "manifestVersion": 1, "minecraft": { "version": "1.20.1" } }"#;
        let read = |p: &str| {
            if p == "manifest.json" {
                Some(body.to_vec())
            } else {
                None
            }
        };
        let (fmt, path) = detect_format(&e, read).unwrap();
        assert_eq!(fmt, ModpackFormat::CurseForge);
        assert_eq!(path, "manifest.json");
    }

    #[test]
    fn manifest_with_addons_is_mcbbs() {
        // 同名 manifest.json,但有 addons → MCBBS。
        let e = entries(&["manifest.json", "overrides/x"]);
        let body = br#"{ "manifestType": "minecraftModpack", "addons": [ { "id": "game", "version": "1.20.1" } ] }"#;
        let read = |p: &str| (p == "manifest.json").then(|| body.to_vec());
        let (fmt, _path) = detect_format(&e, read).unwrap();
        assert_eq!(fmt, ModpackFormat::Mcbbs);
    }

    #[test]
    fn manifest_with_launch_info_is_mcbbs() {
        let e = entries(&["manifest.json"]);
        let body = br#"{ "manifestType": "minecraftModpack", "launchInfo": { "minMemory": 4096 } }"#;
        let read = |p: &str| (p == "manifest.json").then(|| body.to_vec());
        let (fmt, _) = detect_format(&e, read).unwrap();
        assert_eq!(fmt, ModpackFormat::Mcbbs);
    }

    #[test]
    fn manifest_unreadable_falls_back_to_curseforge() {
        // 读不到内容 → 保守按 CurseForge。
        let e = entries(&["manifest.json"]);
        let (fmt, _) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::CurseForge);
    }

    #[test]
    fn detects_multimc_by_mmc_pack() {
        let e = entries(&["mmc-pack.json", "instance.cfg", "minecraft/mods/a.jar"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::MultiMc);
        // mmc-pack.json 与 instance.cfg 同深度,取任一根浅命中均可,但二者都在根。
        assert!(path == "mmc-pack.json" || path == "instance.cfg");
    }

    #[test]
    fn multimc_only_instance_cfg() {
        let e = entries(&["instance.cfg", "minecraft/options.txt"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::MultiMc);
        assert_eq!(path, "instance.cfg");
    }

    #[test]
    fn multimc_beats_modrinth_when_both_present() {
        // 优先级:MultiMC(2)在 Modrinth(3)之前。
        let e = entries(&["mmc-pack.json", "modrinth.index.json"]);
        let (fmt, _) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::MultiMc);
    }

    #[test]
    fn detects_packwiz() {
        let e = entries(&["pack.toml", "index.toml", "mods/sodium.pw.toml"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Packwiz);
        assert_eq!(path, "pack.toml");
    }

    #[test]
    fn detects_technic_by_bin_modpack_jar() {
        let e = entries(&["bin/modpack.jar", "config/a.cfg"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Technic);
        assert_eq!(path, "bin/modpack.jar");
    }

    #[test]
    fn detects_technic_by_bin_version_json_nested() {
        let e = entries(&["Pack/bin/version.json", "Pack/mods/a.jar"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Technic);
        assert_eq!(path, "Pack/bin/version.json");
    }

    #[test]
    fn unknown_returns_none() {
        let e = entries(&["README.md", "mods/a.jar", "config/b.cfg"]);
        assert!(detect_format(&e, no_read).is_none());
    }

    #[test]
    fn nested_root_picks_shallowest_marker() {
        // 包根有 modrinth.index.json,内层 overrides 里也混了一个同名文件:取最浅。
        let e = entries(&[
            "modrinth.index.json",
            "overrides/some/modrinth.index.json",
        ]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Modrinth);
        assert_eq!(path, "modrinth.index.json", "应取根最浅的标记");
    }

    #[test]
    fn backslash_paths_are_normalized() {
        // Windows 风格反斜杠路径也应被识别。
        let e = entries(&["MyPack\\modrinth.index.json"]);
        let (fmt, path) = detect_format(&e, no_read).unwrap();
        assert_eq!(fmt, ModpackFormat::Modrinth);
        assert_eq!(path, "MyPack/modrinth.index.json");
    }
}
