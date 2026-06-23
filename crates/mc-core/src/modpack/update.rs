//! 整合包更新:检查实例来源版本与平台可用版本的差异([`newer_versions`]),并把一个由
//! Modrinth 整合包安装的实例**就地更新**到新版本([`apply_modpack_update`])。
//!
//! 就地更新的核心思路:用导入引擎把新包**覆盖导入**到既有实例(引擎的
//! [`ImportOptions::instance_id`] 已支持指定目标实例),再把「仅旧包有、新包没有」的受管理
//! 文件移入回收站。存档、实例配置、用户自行添加的 mod 都不在整合包索引里,因此天然被保留。

use std::collections::HashSet;
use std::path::Path;

use mc_types::Progress;
use tokio::sync::watch;

use crate::download::{DownloadItem, Downloader};
use crate::error::{CoreError, Result};
use crate::instance::Instance;
use crate::modpack::formats::mrpack::MrpackIndex;
use crate::modpack::import::archive::{StagingDir, ZipArchiveIndex};
use crate::modpack::import::{BlockedFile, ImportEngine, ImportOptions, ImportSource, ManagedPack};
use crate::modplatform::modrinth::VersionDetail;
use crate::paths::GamePaths;

/// `.mrpack` 根级索引文件名。
const MRPACK_INDEX: &str = "modrinth.index.json";

/// 从平台版本列表里挑出比当前安装版本「更新」的那些。
///
/// Modrinth 的版本列表按发布时间倒序(最新在前)。当前实例来源的 `current_version_id`
/// 在列表里定位后,取它**之前**(更新)的所有版本即为可更新项。
///
/// 定位不到(版本被下架,或来源 `version_id` 未知/为 `None`)时返回空 —— 宁可不提示,
/// 也不把整张列表误当成「都比你新」。
pub fn newer_versions(
    versions: Vec<VersionDetail>,
    current_version_id: Option<&str>,
) -> Vec<VersionDetail> {
    let Some(cur) = current_version_id else {
        return Vec::new();
    };
    match versions.iter().position(|v| v.id == cur) {
        Some(pos) => versions.into_iter().take(pos).collect(),
        None => Vec::new(),
    }
}

/// 旧包有、新包没有的受管理文件(版本更新里被移除的 mod/资源),按归一化路径比较。
///
/// 仅比较整合包 `files[]`(受管理的 mods/资源);overrides 与用户自行添加的文件不在索引里,
/// 因此天然不会被当成「被移除」—— 存档、用户 mod 都被保留。返回的是**旧包里的原始路径**
/// (供按其落盘位置清理)。
pub fn compute_pack_diff(old_paths: &[String], new_paths: &[String]) -> Vec<String> {
    let new_set: HashSet<String> = new_paths.iter().map(|p| norm_path(p)).collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut removed: Vec<String> = Vec::new();
    for p in old_paths {
        let n = norm_path(p);
        if !new_set.contains(&n) && seen.insert(n) {
            removed.push(p.clone());
        }
    }
    removed
}

/// 归一化整合包文件路径以做集合比较:反斜杠→`/`,去前导 `./`,去首尾空白。
fn norm_path(p: &str) -> String {
    p.trim().replace('\\', "/").trim_start_matches("./").to_string()
}

/// 整合包就地更新的结果。
#[derive(Debug, Clone)]
pub struct UpdateOutcome {
    /// 被更新的实例 id。
    pub instance_id: String,
    /// 因新版本移除而被清理(移入回收站)的旧包文件相对路径。
    pub removed: Vec<String>,
    /// 仍需用户手动下载的文件(CF blocked;Modrinth 包通常为空)。
    pub blocked: Vec<BlockedFile>,
    /// 下载失败被跳过的可选文件。
    pub skipped_optional: Vec<String>,
}

/// 就地把一个由 Modrinth 整合包安装的实例更新到 `new_mrpack_url` 对应的版本。
///
/// 流程:取旧版索引(尽力)与新版索引算出被移除文件 → 用导入引擎把新包**覆盖导入**到
/// **既有实例**(`ImportOptions.instance_id`)→ 导入成功后,把仅旧包有、新包没有的受管理
/// 文件移入回收站。
///
/// 注意:对既有实例的覆盖导入**非事务性**(引擎仅在新建目录时回滚)。若中途失败,实例处于
/// 旧/新混合态但通常仍可启动,可重试更新;失败时**不**清理任何文件(绝不在未成功导入时删东西)。
/// `index_dl` 仅用于拉取两份索引 `.mrpack`,与引擎内部的下载器相互独立。
#[allow(clippy::too_many_arguments)]
pub async fn apply_modpack_update(
    engine: &ImportEngine,
    index_dl: &Downloader,
    paths: &GamePaths,
    instance_id: &str,
    project_id: &str,
    new_version_id: &str,
    new_mrpack_url: &str,
    old_mrpack_url: Option<&str>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<UpdateOutcome> {
    // 1) 旧版索引尽力获取(取不到就不清理被移除文件,绝不误删用户文件)。
    let old_paths = match old_mrpack_url {
        Some(url) => match fetch_index_paths(index_dl, url).await {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "取旧版整合包索引失败,跳过清理被移除的文件");
                Vec::new()
            }
        },
        None => Vec::new(),
    };
    // 2) 新版索引必须可取(取不到则导入也会失败,提前报错)。
    let new_paths = fetch_index_paths(index_dl, new_mrpack_url).await?;
    let removed = compute_pack_diff(&old_paths, &new_paths);

    // 3) 覆盖导入新包到既有实例(下新文件 + 重铺 overrides + 幂等装核心 + 更新溯源)。
    let mut opts = ImportOptions::new(paths.root().to_path_buf());
    opts.instance_id = Some(instance_id.to_string());
    opts.managed = Some(ManagedPack {
        platform: "modrinth".to_string(),
        project_id: project_id.to_string(),
        version_id: Some(new_version_id.to_string()),
    });
    let outcome = engine
        .import_with_progress(ImportSource::Url(new_mrpack_url.to_string()), opts, progress)
        .await?;

    // 4) 仅在导入成功后,清理被新版移除的受管理文件(移入回收站,可找回)。
    let game_dir = Instance::new(instance_id, paths.root().to_path_buf()).game_dir();
    let mut trashed: Vec<String> = Vec::new();
    for rel in &removed {
        let Some(p) = crate::fs::safe_join(&game_dir, rel) else {
            continue;
        };
        if p.is_file() {
            if trash::delete(&p).is_err() {
                let _ = std::fs::remove_file(&p);
            }
            trashed.push(rel.clone());
        }
    }

    Ok(UpdateOutcome {
        instance_id: outcome.instance_id,
        removed: trashed,
        blocked: outcome.blocked,
        skipped_optional: outcome.skipped_optional,
    })
}

/// 下载一个 `.mrpack` 到临时目录,解析出它**实际会安装**的受管理文件路径
/// (client 支持 + 有下载源,与导入时落盘的文件一致),用于版本间 diff。
async fn fetch_index_paths(dl: &Downloader, url: &str) -> Result<Vec<String>> {
    let staging = StagingDir::new()?;
    let dest = staging.path().join("pack.mrpack");
    dl.download_one(&DownloadItem::new(url.to_string(), dest.clone(), None, None))
        .await?;
    Ok(index_install_paths(&read_mrpack_index(&dest)?))
}

/// 从一个已下载的 `.mrpack` 读出并解析 `modrinth.index.json`。
fn read_mrpack_index(archive_path: &Path) -> Result<MrpackIndex> {
    let mut zip = ZipArchiveIndex::open(archive_path)?;
    let bytes = zip
        .read_small_owned(MRPACK_INDEX)
        .ok_or_else(|| CoreError::other(".mrpack 缺少 modrinth.index.json"))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| CoreError::Parse { what: MRPACK_INDEX.into(), source: e })
}

/// 整合包索引里**实际会落盘**的受管理文件相对路径(client 支持 + 有下载源)。
fn index_install_paths(index: &MrpackIndex) -> Vec<String> {
    index
        .files
        .iter()
        .filter(|f| f.client_supported() && !f.downloads.is_empty())
        .map(|f| f.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modpack::formats::mrpack::{
        EnvSupport, MrpackDependencies, MrpackEnv, MrpackFile, MrpackHashes,
    };

    fn v(id: &str) -> VersionDetail {
        VersionDetail {
            id: id.to_string(),
            version_number: id.to_string(),
            name: id.to_string(),
            version_type: "release".to_string(),
            game_versions: vec![],
            loaders: vec![],
            date_published: String::new(),
            downloads: 0,
            changelog: String::new(),
            mrpack_url: None,
            mrpack_filename: None,
            file_size: None,
        }
    }

    #[test]
    fn takes_versions_before_current() {
        // newest-first: c, b, a;当前是 b → 只有 c 更新。
        let r = newer_versions(vec![v("c"), v("b"), v("a")], Some("b"));
        assert_eq!(r.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["c"]);
    }

    #[test]
    fn current_is_newest_means_none() {
        assert!(newer_versions(vec![v("c"), v("b"), v("a")], Some("c")).is_empty());
    }

    #[test]
    fn unknown_or_missing_current_returns_empty() {
        assert!(newer_versions(vec![v("c"), v("b")], Some("zzz")).is_empty());
        assert!(newer_versions(vec![v("c")], None).is_empty());
    }

    #[test]
    fn diff_finds_removed_managed_files() {
        let old = vec!["mods/a.jar".into(), "mods/b.jar".into(), "config/x.toml".into()];
        let new = vec!["mods/a.jar".into(), "config/x.toml".into()];
        assert_eq!(compute_pack_diff(&old, &new), vec!["mods/b.jar".to_string()]);
    }

    #[test]
    fn diff_ignores_added_and_kept() {
        // 新增(new.jar)与保留(a.jar)都不算「被移除」。
        let old = vec!["mods/a.jar".into()];
        let new = vec!["mods/a.jar".into(), "mods/new.jar".into()];
        assert!(compute_pack_diff(&old, &new).is_empty());
    }

    #[test]
    fn diff_normalizes_separators() {
        // a 经归一化(反斜杠、前导 ./)视为保留;只有 b 被移除,返回其原始路径串。
        let old = vec!["mods\\a.jar".into(), "./mods/b.jar".into()];
        let new = vec!["mods/a.jar".into()];
        assert_eq!(compute_pack_diff(&old, &new), vec!["./mods/b.jar".to_string()]);
    }

    #[test]
    fn install_paths_skip_server_only_and_sourceless() {
        // client unsupported 的纯服务端文件、以及无下载源的文件都不计入安装路径。
        let index = MrpackIndex {
            format_version: 1,
            game: "minecraft".to_string(),
            version_id: "1".to_string(),
            name: "P".to_string(),
            summary: None,
            dependencies: MrpackDependencies {
                minecraft: Some("1.20.1".to_string()),
                ..Default::default()
            },
            files: vec![
                MrpackFile {
                    path: "mods/keep.jar".to_string(),
                    hashes: MrpackHashes { sha512: "h".to_string(), sha1: None },
                    env: None,
                    downloads: vec!["https://cdn.modrinth.com/keep.jar".to_string()],
                    file_size: None,
                },
                MrpackFile {
                    path: "mods/server-only.jar".to_string(),
                    hashes: MrpackHashes { sha512: "h2".to_string(), sha1: None },
                    env: Some(MrpackEnv {
                        client: EnvSupport::Unsupported,
                        server: EnvSupport::Required,
                    }),
                    downloads: vec!["https://cdn.modrinth.com/srv.jar".to_string()],
                    file_size: None,
                },
                MrpackFile {
                    path: "mods/no-source.jar".to_string(),
                    hashes: MrpackHashes { sha512: "h3".to_string(), sha1: None },
                    env: None,
                    downloads: vec![],
                    file_size: None,
                },
            ],
        };
        assert_eq!(index_install_paths(&index), vec!["mods/keep.jar".to_string()]);
    }
}
