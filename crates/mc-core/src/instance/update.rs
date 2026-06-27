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

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};

use crate::download::checksum::sha1_file;
use crate::download::{DownloadItem, Downloader};
use crate::error::Result;
use crate::instance::{list_instances, list_mods, Instance, ModInfo};
use crate::modplatform::modrinth::ModrinthApi;
use crate::modplatform::{ProjectVersion, VersionFile};
use crate::paths::GamePaths;

/// 一个可用的 mod 更新。携带应用更新所需的全部信息(下载地址 / 校验 / 目标文件名),
/// 这样 UI 拿到后无需再查一次即可直接调用 [`apply_mod_update`]。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

    out.sort_by_key(|a| a.name.to_ascii_lowercase());
    Ok(out)
}

/// 批量更新检查里**单个实例**的结果:有多少个 mod 可更新、整合包本身是否有新版本。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InstanceUpdateInfo {
    /// 实例 id。
    pub instance_id: String,
    /// 可更新的已启用 mod 数量(`check_mod_updates` 结果的长度)。
    pub mod_updates: u32,
    /// 该实例(由 Modrinth 整合包安装)是否有比当前来源版本更新的整合包版本。
    pub modpack_update: bool,
}

/// 一次性检查 `root` 下**所有实例**的可用更新(每实例:已启用 mod 的更新数 + 整合包是否有新版)。
///
/// 网络密集(逐实例打 Modrinth),因此**只应按需调用**,不要在启动时自动跑。每个实例的检查相互
/// 独立并以**有界并发**(`CONCURRENCY`)推进,避免 N 个实例串行;**单实例失败被吞掉**(记 warn 并
/// 跳过,不影响其它实例),所以返回的列表可能短于实例总数。
///
/// 只返回**至少有一项更新**的实例(`mod_updates > 0` 或 `modpack_update`);无更新的实例不在结果里,
/// 前端据此点亮卡片角标即可。
pub async fn check_all_updates(api: &ModrinthApi, paths: &GamePaths) -> Vec<InstanceUpdateInfo> {
    /// 同时在飞的实例检查数上限:既跑满 Modrinth 的吞吐,又不至于一次性打爆它。
    const CONCURRENCY: usize = 6;

    let instances = list_instances(paths);
    stream::iter(instances)
        .map(|summary| async move {
            match check_one(api, paths, &summary).await {
                Ok(info) if info.mod_updates > 0 || info.modpack_update => Some(info),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(instance = %summary.id, error = %e, "批量更新检查:跳过该实例");
                    None
                }
            }
        })
        .buffer_unordered(CONCURRENCY)
        .filter_map(|x| async move { x })
        .collect()
        .await
}

/// 单实例检查:mod 更新数 + 整合包是否有新版。任一网络步骤失败即整体 `Err`(由调用方吞掉)。
async fn check_one(
    api: &ModrinthApi,
    paths: &GamePaths,
    summary: &mc_types::InstanceSummary,
) -> Result<InstanceUpdateInfo> {
    let inst = Instance::new(&summary.id, paths.root().to_path_buf());

    let mod_updates =
        check_mod_updates(api, &inst, &summary.mc_version, summary.loader.as_str()).await?.len() as u32;

    // 整合包更新:仅对「Modrinth 整合包来源」的实例有意义,其余直接视作无整合包更新。
    let modpack_update = match inst.load_config()?.source {
        Some(src) if src.provider == "modrinth" => {
            let all = api.version_details(&src.project_id).await?;
            !crate::modpack::update::newer_versions(all, src.version_id.as_deref()).is_empty()
        }
        _ => false,
    };

    Ok(InstanceUpdateInfo { instance_id: summary.id.clone(), mod_updates, modpack_update })
}

/// 应用一个更新:把新版本文件下载进 `mods/`,再删掉旧文件(文件名不同才需删,
/// 同名则下载已覆盖)。删除走回收站(可恢复),与 mod 删除一致。
pub async fn apply_mod_update(inst: &Instance, dl: &Downloader, update: &ModUpdate) -> Result<()> {
    // new_file_name 源自平台 API(不可信):单一路径段校验,防止写出 mods/ 之外。
    let dest = crate::fs::resolve_segment(&inst.mods_dir(), &update.new_file_name)?;
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
    use crate::modplatform::VersionFile;

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
            }],
            dependencies: vec![],
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
