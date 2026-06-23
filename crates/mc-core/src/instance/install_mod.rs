//! Mod 安装 + 依赖解析。
//!
//! 本模块把 [`crate::modplatform`] 的统一版本模型(`ProjectVersion` /
//! `VersionFile` / `Dependency`)落地成"把 jar 下到实例 `mods/` 目录"的动作,
//! 并在 `resolve_deps` 打开时递归安装 required 依赖。
//!
//! 设计要点:
//! - **版本选择是纯逻辑、可单测**:[`pick_version`] / [`primary_file`] 不联网、
//!   不碰磁盘,只对内存里的版本列表做过滤,故核心选择策略能被单元测试覆盖。
//! - **依赖图防环 + 去重**:递归用一个 `already-visited` 的 `HashSet<String>`
//!   (按 project_id 去重)防止依赖环(A→B→A)与重复下载(钻石依赖)导致的
//!   无限递归 / 冗余请求。已访问过的依赖记入 `satisfied`。
//! - **async 递归**:Rust 的 async fn 不能直接自递归(返回 `impl Future` 会形成
//!   无限大的类型),故把递归体写成返回 `Pin<Box<dyn Future>>` 的内部函数,
//!   在调用点 `Box::pin` 装箱,打破类型递归。
//! - **容错语义**:找得到兼容版本并下载成功 → 记入 `installed`;依赖已在本次安装
//!   过程中处理过 → 记入 `satisfied`;找不到兼容(mc_version+loader)版本 →
//!   记入 `unresolved`(不报错中断,让主 mod 仍然装上,缺失项交给上层提示)。
//!   只有真正的 IO / 网络 / 下载错误才向上 `?` 传播为 [`crate::error::CoreError`]。

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;

use serde::Serialize;

use crate::download::{DownloadItem, Downloader};
use crate::error::Result;
use crate::instance::Instance;
use crate::modplatform::modrinth::ModrinthApi;
use crate::modplatform::{ProjectVersion, VersionFile};

/// 一个已成功安装的 mod 记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstalledMod {
    /// Modrinth project id。
    pub project_id: String,
    /// 落盘到 `mods/` 的文件名。
    pub file_name: String,
}

/// 一次 [`install_mod`] 调用的结果汇总。
///
/// 三个列表互斥地反映依赖处理的三种归宿:
/// - `installed`:本次新下载落盘的(含主 mod 与其 required 依赖)。
/// - `satisfied`:依赖的 project_id 在本次安装过程中已被处理(去重/防环命中),
///   无需重复下载。
/// - `unresolved`:声明为 required 的依赖,但在该 mc_version+loader 下找不到任何
///   兼容版本,需上层提示用户手动处理。
#[derive(Debug, Clone, Default, Serialize)]
pub struct InstallReport {
    pub installed: Vec<InstalledMod>,
    pub satisfied: Vec<String>,
    pub unresolved: Vec<String>,
    /// 所装版本声明为 `incompatible` 的项目 id(冲突)。不阻断安装,交上层提示用户检查。
    #[serde(default)]
    pub incompatible: Vec<String>,
}

/// 把某版本声明为 `incompatible` 的依赖 project_id 收进 `report.incompatible`(去重)。
/// 仅记账、不阻断安装——是否真冲突由用户结合已装内容判断。
fn collect_incompatible(version: &ProjectVersion, report: &mut InstallReport) {
    for d in &version.dependencies {
        if d.dependency_type == "incompatible" {
            if let Some(pid) = &d.project_id {
                if !report.incompatible.contains(pid) {
                    report.incompatible.push(pid.clone());
                }
            }
        }
    }
}

/// 从一组版本里挑出第一个同时兼容 `mc_version` 与 `loader` 的版本。
///
/// Modrinth 的版本列表按发布时间新→旧返回,故"第一个匹配"即"最新的兼容版本",
/// 这是安装新 mod 时最合理的默认选择。比较对 mc_version 精确匹配(元素相等),
/// loader 同理——上层若想要更松的语义(如允许快照)应在传入前自行过滤。
pub fn pick_version<'a>(
    versions: &'a [ProjectVersion],
    mc_version: &str,
    loader: &str,
) -> Option<&'a ProjectVersion> {
    // Quilt 实例接受 fabric 版本(accepted_loaders 展开),其余 loader 即自身精确匹配。
    let accepted = crate::modplatform::accepted_loaders(loader);
    versions.iter().find(|v| {
        v.game_versions.iter().any(|g| g == mc_version)
            && v.loaders
                .iter()
                .any(|l| accepted.iter().any(|a| a.eq_ignore_ascii_case(l)))
    })
}

/// 取一个版本的主文件:优先 `primary == true`,否则退回第一个文件。
///
/// 与 [`ProjectVersion::primary_file`] 语义一致,这里独立成自由函数以满足模块
/// 接口要求并便于单测;无文件时返回 `None`。
pub fn primary_file(v: &ProjectVersion) -> Option<&VersionFile> {
    v.files.iter().find(|f| f.primary).or_else(|| v.files.first())
}

/// 把某个具体版本的主文件下载到实例的 `mods/` 目录,返回落盘文件名。
///
/// sha1 取自 [`VersionFile::sha1`](crate::modplatform::VersionFile::sha1);提供时
/// [`Downloader::download_one`] 会做强校验。该版本没有任何文件时返回
/// [`crate::error::CoreError::Other`]——一个无文件的版本无法安装,属于数据异常。
pub async fn install_mod_version(
    inst: &Instance,
    dl: &Downloader,
    v: &ProjectVersion,
) -> Result<String> {
    let file = primary_file(v).ok_or_else(|| {
        crate::error::CoreError::other(format!(
            "project version {} ({}) has no downloadable file",
            v.id, v.version_number
        ))
    })?;

    let dest = inst.mods_dir().join(&file.filename);

    // download_one 内部会建父目录(mods/)、流式下载并按 sha1 校验。
    dl.download_one(&DownloadItem {
        url: file.url.clone(),
        path: dest,
        sha1: file.sha1.clone(),
        size: file.size,
        ..Default::default()
    })
    .await?;

    // 装好新版本后,清掉实例里同一个 mod 的旧 jar(否则同 modId 重复会让游戏崩溃)。
    // 失败不阻断安装本身 —— 文件已经下好,清理是尽力而为。
    let _ = crate::instance::mods::remove_superseded(inst, &file.filename);

    Ok(file.filename.clone())
}

/// 安装一个**指定版本**的 mod,并解析它声明的 required 依赖(取各依赖最新兼容版本)。
///
/// 与 [`install_mod`](crate::instance::install_mod)「装最新版 + 解析依赖」对称:用户从版本
/// 列表显式选版安装时,也应把缺的前置(如 Fabric API)一并补上,而不是装个孤立的 jar。
/// 主文件先装(其 `project_id` 未知,留空);随后对每个 required 依赖走与 install_mod 相同的
/// 递归安装(防环 / 去重 / unresolved 记账)。
pub async fn install_mod_version_with_deps(
    api: &ModrinthApi,
    dl: &Downloader,
    inst: &Instance,
    version: &ProjectVersion,
    mc_version: &str,
    loader: &str,
) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    let mut visited: HashSet<String> = HashSet::new();

    let file_name = install_mod_version(inst, dl, version).await?;
    report.installed.push(InstalledMod {
        project_id: String::new(),
        file_name,
    });
    collect_incompatible(version, &mut report);

    let dep_ids: Vec<String> = version
        .dependencies
        .iter()
        .filter(|d| d.dependency_type == "required")
        .filter_map(|d| d.project_id.clone())
        .collect();

    for dep_id in dep_ids {
        install_rec(
            api, dl, inst, &dep_id, mc_version, loader, true, &mut visited, &mut report,
        )
        .await?;
    }

    Ok(report)
}

/// 安装某个 Modrinth 项目(及可选的 required 依赖)到实例的 `mods/` 目录。
///
/// 流程:`get_versions(project_id, mc, loader)` → [`pick_version`] →
/// [`install_mod_version`]。`resolve_deps` 为真时,对所选版本的
/// `dependency_type == "required"` 且带 `project_id` 的依赖递归安装。
///
/// 找不到主 mod 的兼容版本时,把它记入 `unresolved` 并返回(不报错):上层据此
/// 可提示"该 mod 不支持当前 mc/loader"。网络 / 下载 / IO 故障才向上传播。
pub async fn install_mod(
    api: &ModrinthApi,
    dl: &Downloader,
    inst: &Instance,
    project_id: &str,
    mc_version: &str,
    loader: &str,
    resolve_deps: bool,
) -> Result<InstallReport> {
    let mut report = InstallReport::default();
    // 已处理集合:从根 mod 起即记入,既防环也避免根 mod 被它自身的依赖链重复安装。
    let mut visited: HashSet<String> = HashSet::new();

    install_rec(
        api,
        dl,
        inst,
        project_id,
        mc_version,
        loader,
        resolve_deps,
        &mut visited,
        &mut report,
    )
    .await?;

    Ok(report)
}

/// 递归安装实现。
///
/// 写成"返回 `Pin<Box<dyn Future>>` 的同步函数"而非 `async fn`,以打破 async 自
/// 递归带来的无限类型递归;内部用 `async move` 块产生 future 再 `Box::pin`。
/// 借用 `visited` / `report` 的 `&mut` 在整个 future 生命周期内有效,故给 future
/// 标注 `+ 'a` 把它们的生命周期绑进返回类型。
#[allow(clippy::too_many_arguments)]
fn install_rec<'a>(
    api: &'a ModrinthApi,
    dl: &'a Downloader,
    inst: &'a Instance,
    project_id: &'a str,
    mc_version: &'a str,
    loader: &'a str,
    resolve_deps: bool,
    visited: &'a mut HashSet<String>,
    report: &'a mut InstallReport,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        // 去重 / 防环:已处理过的 project_id 直接记入 satisfied 并返回。
        // insert 返回 false 代表此前已存在。
        if !visited.insert(project_id.to_string()) {
            report.satisfied.push(project_id.to_string());
            return Ok(());
        }

        // 拉取该项目在目标 mc/loader 下的版本列表(服务端已按 loader/版本过滤,
        // 这里再用 pick_version 精确取第一个兼容项以防服务端返回宽松结果)。
        let versions = api
            .get_versions(project_id, Some(mc_version), Some(loader))
            .await?;

        let chosen = match pick_version(&versions, mc_version, loader) {
            Some(v) => v,
            None => {
                // 没有兼容版本:记为 unresolved,不阻断其余安装。
                report.unresolved.push(project_id.to_string());
                return Ok(());
            }
        };

        let file_name = install_mod_version(inst, dl, chosen).await?;
        report.installed.push(InstalledMod {
            project_id: project_id.to_string(),
            file_name,
        });
        collect_incompatible(chosen, report);

        if resolve_deps {
            // 只处理 required 且带 project_id 的依赖;optional/incompatible/embedded
            // 与仅指定 version_id(无 project_id,无法定位项目)的依赖在此跳过。
            // 先把依赖 id 收集出来,避免在 await 期间持有对 chosen(借自 versions)的借用。
            let dep_ids: Vec<String> = chosen
                .dependencies
                .iter()
                .filter(|d| d.dependency_type == "required")
                .filter_map(|d| d.project_id.clone())
                .collect();

            for dep_id in dep_ids {
                install_rec(
                    api,
                    dl,
                    inst,
                    &dep_id,
                    mc_version,
                    loader,
                    resolve_deps,
                    visited,
                    report,
                )
                .await?;
            }
        }

        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modplatform::{Dependency, ProjectVersion, VersionFile};

    /// 便捷构造一个版本,只填测试关心的字段。
    fn version(
        id: &str,
        game_versions: &[&str],
        loaders: &[&str],
        files: Vec<VersionFile>,
    ) -> ProjectVersion {
        ProjectVersion {
            id: id.into(),
            name: id.into(),
            version_number: id.into(),
            game_versions: game_versions.iter().map(|s| s.to_string()).collect(),
            loaders: loaders.iter().map(|s| s.to_string()).collect(),
            files,
            dependencies: Vec::new(),
        }
    }

    fn file(name: &str, primary: bool) -> VersionFile {
        VersionFile {
            url: format!("https://example.com/{name}"),
            filename: name.into(),
            primary,
            ..Default::default()
        }
    }

    #[test]
    fn pick_version_takes_first_compatible() {
        // 列表按新→旧排列;最新的兼容版本应被选中(第一个匹配)。
        let versions = vec![
            // 最新,但只支持 1.21 → 不匹配 1.20.1。
            version("newest", &["1.21"], &["fabric"], vec![]),
            // 次新,支持 1.20.1 + fabric → 期望命中这个。
            version("good-new", &["1.20.1", "1.20"], &["fabric"], vec![]),
            // 更旧的也兼容,但因排在后面不应被选。
            version("good-old", &["1.20.1"], &["fabric", "quilt"], vec![]),
        ];
        let picked = pick_version(&versions, "1.20.1", "fabric").unwrap();
        assert_eq!(picked.id, "good-new");
    }

    #[test]
    fn pick_version_requires_both_game_and_loader() {
        let versions = vec![
            // 正确 mc 版本但 loader 是 forge → 不匹配 fabric。
            version("wrong-loader", &["1.20.1"], &["forge"], vec![]),
            // 正确 loader 但 mc 版本是 1.19.2 → 不匹配 1.20.1。
            version("wrong-mc", &["1.19.2"], &["fabric"], vec![]),
        ];
        assert!(pick_version(&versions, "1.20.1", "fabric").is_none());
    }

    #[test]
    fn pick_version_empty_list_is_none() {
        let versions: Vec<ProjectVersion> = Vec::new();
        assert!(pick_version(&versions, "1.20.1", "fabric").is_none());
    }

    #[test]
    fn quilt_instance_accepts_fabric_only_mod() {
        // 一个只发布 fabric 版本的 mod:quilt 实例也应能选中它(Quilt 兼容 Fabric)。
        let versions = vec![version("fab-only", &["1.20.1"], &["fabric"], vec![])];
        assert_eq!(pick_version(&versions, "1.20.1", "quilt").unwrap().id, "fab-only");
        // 反之 fabric 实例不接受 quilt-only 版本。
        let quilt_only = vec![version("q-only", &["1.20.1"], &["quilt"], vec![])];
        assert!(pick_version(&quilt_only, "1.20.1", "fabric").is_none());
    }

    #[test]
    fn primary_file_prefers_primary_flag() {
        let v = version(
            "v",
            &["1.20.1"],
            &["fabric"],
            vec![file("sources.jar", false), file("mod.jar", true)],
        );
        // 即便 primary 文件不在首位,也应被优先选中。
        assert_eq!(primary_file(&v).unwrap().filename, "mod.jar");
    }

    #[test]
    fn primary_file_falls_back_to_first() {
        let v = version(
            "v",
            &["1.20.1"],
            &["fabric"],
            vec![file("first.jar", false), file("second.jar", false)],
        );
        // 没有任何 primary 标记 → 退回第一个文件。
        assert_eq!(primary_file(&v).unwrap().filename, "first.jar");
    }

    #[test]
    fn primary_file_none_when_no_files() {
        let v = version("v", &["1.20.1"], &["fabric"], vec![]);
        assert!(primary_file(&v).is_none());
    }

    #[test]
    fn report_default_is_empty() {
        let r = InstallReport::default();
        assert!(r.installed.is_empty());
        assert!(r.satisfied.is_empty());
        assert!(r.unresolved.is_empty());
    }

    #[test]
    fn report_serializes_to_json() {
        // InstallReport 派生 Serialize,需可经 serde_json 回传给前端。
        let report = InstallReport {
            installed: vec![InstalledMod {
                project_id: "AABBCC".into(),
                file_name: "sodium.jar".into(),
            }],
            satisfied: vec!["fabric-api".into()],
            unresolved: vec!["missing-lib".into()],
            incompatible: vec!["conflicting-mod".into()],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["installed"][0]["project_id"], "AABBCC");
        assert_eq!(json["installed"][0]["file_name"], "sodium.jar");
        assert_eq!(json["satisfied"][0], "fabric-api");
        assert_eq!(json["unresolved"][0], "missing-lib");
    }

    #[test]
    fn required_dependency_filter_logic() {
        // 验证"只取 required 且带 project_id"的依赖筛选规则(对应递归里的过滤)。
        let deps = vec![
            Dependency {
                project_id: Some("req-with-id".into()),
                version_id: None,
                dependency_type: "required".into(),
            },
            Dependency {
                project_id: Some("optional-dep".into()),
                version_id: None,
                dependency_type: "optional".into(),
            },
            Dependency {
                // required 但只有 version_id、无 project_id → 应被跳过(无法定位项目)。
                project_id: None,
                version_id: Some("ver-only".into()),
                dependency_type: "required".into(),
            },
            Dependency {
                project_id: Some("embedded-dep".into()),
                version_id: None,
                dependency_type: "embedded".into(),
            },
        ];

        let selected: Vec<String> = deps
            .iter()
            .filter(|d| d.dependency_type == "required")
            .filter_map(|d| d.project_id.clone())
            .collect();

        assert_eq!(selected, vec!["req-with-id".to_string()]);
    }
}
