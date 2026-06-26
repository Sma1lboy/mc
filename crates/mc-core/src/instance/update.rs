//! Mod 更新检查 —— 对实例里已装的 mod,问 Modrinth「在当前 loader / 游戏版本下有没有更新的版本」。
//!
//! 原理:每个 mod jar 的 sha1 唯一标识它在 Modrinth 上的那一个版本文件。把所有已启用
//! mod 的 sha1 一次性 POST 到 `/version_files/update`(带 loaders / game_versions 过滤),
//! Modrinth 直接回传每个哈希对应项目的**最新兼容版本**。若回传版本的主文件 sha1 与我们
//! 传上去的不同,即说明有更新。
//!
//! 只检查**已启用**的 mod:停用的 mod 用户已主动关掉,替它拉更新没有意义,也避免把
//! `.disabled` 文件误当作活跃 mod。本地反查不到(非 Modrinth 来源、手动放入)的 jar 会
//! 在响应里缺席,自然被跳过,不报错。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::download::checksum::sha1_file;
use crate::download::{DownloadItem, Downloader};
use crate::error::Result;
use crate::instance::{list_mods, Instance, ModInfo};
use crate::modplatform::modrinth::ModrinthApi;
use crate::modplatform::{ProjectVersion, VersionFile};

/// 一个可用的 mod 更新。携带应用更新所需的全部信息(下载地址 / 校验 / 目标文件名),
/// 这样 UI 拿到后无需再查一次即可直接调用 [`apply_mod_update`]。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModUpdate {
    /// 当前磁盘上的文件名(启用态 `.jar`),应用更新后会被替换/删除。
    pub file_name: String,
    /// 展示名(取本地解析到的 mod 名,读不到时为文件名)。
    pub name: String,
    /// 当前版本号(本地元数据;可能缺失)。
    pub current_version: Option<String>,
    /// 最新版本号(Modrinth `version_number`)。
    pub new_version: String,
    /// 最新版本主文件的文件名(下载后的落盘名)。
    pub new_file_name: String,
    /// 下载地址。
    pub url: String,
    /// 最新文件 sha1(下载校验用;Modrinth 通常都提供)。
    pub sha1: Option<String>,
    /// 最新文件大小(字节)。
    pub size: Option<u64>,
}

/// 纯函数:给定已装文件的 sha1 与 Modrinth 回传的「最新版本」,判断是否构成更新。
///
/// 规则:取最新版本的主文件;若它的 sha1 与已装 sha1 相同,说明本地已是最新 → 返回 `None`;
/// 否则返回该主文件(调用方据此构造 [`ModUpdate`])。比较大小写不敏感(十六进制)。
/// 拆成纯函数以便脱离网络做单测。
fn updated_file<'a>(installed_sha1: &str, latest: &'a ProjectVersion) -> Option<&'a VersionFile> {
    let file = latest.primary_file()?;
    match &file.sha1 {
        // 同一文件(sha1 相同)→ 已是最新,无更新。
        Some(s) if s.eq_ignore_ascii_case(installed_sha1) => None,
        // sha1 不同(或最新文件未提供 sha1,保守视作有更新)→ 有更新。
        _ => Some(file),
    }
}

/// 检查实例里所有**已启用** mod 是否有更新。返回的列表已按展示名稳定排序。
///
/// 步骤:列举启用 mod → 算 sha1 → 一次 `/version_files/update` 批量查询 → 逐个用
/// [`updated_file`] 判断。本地算不出 sha1(读失败)或 Modrinth 查不到的条目被跳过。
pub async fn check_mod_updates(
    api: &ModrinthApi,
    inst: &Instance,
    mc_version: &str,
    loader: &str,
) -> Result<Vec<ModUpdate>> {
    // 收集启用 mod 的 sha1 → 本地元数据。
    let mut by_hash: HashMap<String, ModInfo> = HashMap::new();
    let mut hashes: Vec<String> = Vec::new();
    for m in list_mods(inst).into_iter().filter(|m| m.enabled) {
        let path = inst.mods_dir().join(&m.file_name);
        if let Ok(hash) = sha1_file(&path) {
            hashes.push(hash.clone());
            by_hash.insert(hash, m);
        }
    }
    if hashes.is_empty() {
        return Ok(Vec::new());
    }

    // Quilt 实例同时接受 fabric mod 的更新;其余 loader 即自身。
    let loaders = crate::modplatform::accepted_loaders(loader);
    let latest = api
        .latest_versions_from_hashes(&hashes, "sha1", &loaders, &[mc_version.to_string()])
        .await?;

    let mut out: Vec<ModUpdate> = Vec::new();
    for (hash, info) in by_hash {
        let Some(version) = latest.get(&hash) else {
            continue; // Modrinth 不认识这个文件,跳过。
        };
        let Some(file) = updated_file(&hash, version) else {
            continue; // 已是最新。
        };
        out.push(ModUpdate {
            file_name: info.file_name,
            name: info.name,
            current_version: info.version,
            new_version: version.version_number.clone(),
            new_file_name: file.filename.clone(),
            url: file.url.clone(),
            sha1: file.sha1.clone(),
            size: file.size,
        });
    }

    out.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
    Ok(out)
}

/// 应用一个更新:把新版本文件下载进 `mods/`,再删掉旧文件(文件名不同才需删,
/// 同名则下载已覆盖)。删除走回收站(可恢复),与 mod 删除一致。
pub async fn apply_mod_update(inst: &Instance, dl: &Downloader, update: &ModUpdate) -> Result<()> {
    let dest = inst.mods_dir().join(&update.new_file_name);
    dl.download_one(&DownloadItem {
        url: update.url.clone(),
        path: dest,
        sha1: update.sha1.clone(),
        size: update.size,
        ..Default::default()
    })
    .await?;

    // 新旧文件名不同:删除旧文件(同名时上一步的下载已原样覆盖,无需删)。
    if update.new_file_name != update.file_name {
        crate::instance::mods::delete_mod(inst, &update.file_name)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modplatform::{ProjectSideSupport, VersionFile};

    fn version_with_primary(sha1: &str, filename: &str) -> ProjectVersion {
        ProjectVersion {
            id: "v1".into(),
            name: "Latest".into(),
            version_number: "2.0.0".into(),
            game_versions: vec!["1.20.1".into()],
            loaders: vec!["fabric".into()],
            files: vec![VersionFile {
                url: "https://example/file.jar".into(),
                filename: filename.into(),
                sha1: Some(sha1.into()),
                sha512: None,
                size: Some(123),
                primary: true,
                client_side: ProjectSideSupport::Unknown,
                server_side: ProjectSideSupport::Unknown,
            }],
            dependencies: vec![],
            client_side: ProjectSideSupport::Unknown,
            server_side: ProjectSideSupport::Unknown,
        }
    }

    #[test]
    fn same_sha1_is_not_an_update() {
        let v = version_with_primary("AABBCC", "mod-2.0.0.jar");
        // 大小写不敏感:本地小写、远端大写,应判定为同一文件 → 无更新。
        assert!(updated_file("aabbcc", &v).is_none());
    }

    #[test]
    fn different_sha1_is_an_update() {
        let v = version_with_primary("ffffff", "mod-2.0.0.jar");
        let file = updated_file("aabbcc", &v).expect("应判定为有更新");
        assert_eq!(file.filename, "mod-2.0.0.jar");
    }

    #[test]
    fn missing_remote_sha1_is_treated_as_update() {
        let mut v = version_with_primary("ignored", "mod.jar");
        v.files[0].sha1 = None;
        // 远端未给 sha1:保守视作有更新(不漏报)。
        assert!(updated_file("aabbcc", &v).is_some());
    }

    #[test]
    fn no_primary_file_means_no_update() {
        let mut v = version_with_primary("x", "y");
        v.files.clear();
        assert!(updated_file("aabbcc", &v).is_none());
    }
}
