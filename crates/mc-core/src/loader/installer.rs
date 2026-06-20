//! Shared helper for Forge/NeoForge installation. Both ship an installer jar
//! that, run headlessly with `--installClient`, downloads their libraries, runs
//! the binary-patch processors, and writes a `versions/<id>/` entry that
//! `inheritsFrom` vanilla. We then just treat that as another component.
//!
//! Running the official installer is the robust cross-version approach: the
//! processor pipeline (1.13+) is too involved to reimplement.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tokio::sync::watch;

use mc_types::{ManifestVersion, Progress};

use crate::download::{DownloadItem, Downloader};
use crate::error::{CoreError, IoResultExt, Result};
use crate::launch;
use crate::paths::{ensure_dir, GamePaths};
use crate::version::RuntimeContext;

/// Names of the version directories currently present.
fn version_dirs(paths: &GamePaths) -> HashSet<String> {
    let mut set = HashSet::new();
    if let Ok(entries) = std::fs::read_dir(paths.versions_dir()) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                if let Some(name) = e.file_name().to_str() {
                    set.insert(name.to_string());
                }
            }
        }
    }
    set
}

/// Download the installer jar, run it headlessly against the game root, and
/// return the id of the version directory it created.
pub async fn run_installer(
    dl: &Downloader,
    paths: &GamePaths,
    installer_url: &str,
    java_path: &Path,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("下载 loader 安装器"));
    }

    ensure_dir(paths.root())?;
    // Forge/NeoForge installers refuse to run without launcher_profiles.json.
    crate::fs::ensure_launcher_profiles(paths.root())?;

    let installer_path = paths.root().join("loader-installer.jar");
    dl.download_one(&DownloadItem::new(
        installer_url.to_string(),
        installer_path.clone(),
        None,
        None,
    ))
    .await?;

    let before = version_dirs(paths);

    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("运行安装器(可能耗时)"));
    }
    let output = tokio::process::Command::new(java_path)
        .arg("-jar")
        .arg(&installer_path)
        .arg("--installClient")
        .arg(paths.root())
        .current_dir(paths.root())
        .output()
        .await
        .map_err(|e| CoreError::Launch(format!("无法运行安装器: {e}")))?;

    // Best-effort cleanup of the installer jar regardless of outcome.
    let _ = std::fs::remove_file(&installer_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let tail: String = stderr.chars().chain(stdout.chars()).rev().take(400).collect::<String>().chars().rev().collect();
        return Err(CoreError::Launch(format!("安装器失败: {tail}")));
    }

    // The new version directory is whatever appeared that wasn't there before.
    let after = version_dirs(paths);
    let mut created: Vec<String> = after.difference(&before).cloned().collect();
    created.sort();
    created
        .into_iter()
        // ignore the vanilla dir if it happened to be created in the same run
        .find(|id| paths.version_json(id).exists())
        .ok_or_else(|| CoreError::other("安装器未生成版本目录(可能需要图形环境或网络)"))
}

/// Find any usable Java executable for running an installer (installers run on
/// Java 8+; the exact game-major doesn't matter here).
pub async fn any_java() -> Option<std::path::PathBuf> {
    let installs = crate::java::detect_all().await;
    // Prefer the newest; installers are forward-compatible.
    installs.into_iter().max_by_key(|j| j.version.clone()).map(|j| j.path)
}

/// Verify a freshly written version json parses (sanity check post-install).
pub fn verify_installed(paths: &GamePaths, id: &str) -> Result<()> {
    let raw = std::fs::read_to_string(paths.version_json(id)).with_path(paths.version_json(id))?;
    crate::version::VersionJson::parse(&raw)
        .map(|_| ())
        .map_err(|e| CoreError::Parse { what: format!("installed version {id}"), source: e })
}

// ===========================================================================
// 各 loader 共用的安装编排步骤
// ---------------------------------------------------------------------------
// fabric/forge/quilt/neoforge 此前各自重复了「确保原版」「收尾确保库」两段;
// forge/neoforge 还各重复了「挑 Java → 跑 installer → 校验」。统一到这里,
// 每个 loader 文件只保留自己真正不同的部分(meta URL / 版本解析 / installer URL)。
// ===========================================================================

/// 确保原版 MC 已在磁盘上(loader profile `inheritsFrom` 它);缺失则装一遍。
pub async fn ensure_vanilla(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    vanilla_entry: &ManifestVersion,
    progress: &Option<watch::Sender<Progress>>,
) -> Result<()> {
    if !paths.version_json(mc_version).exists() {
        if let Some(tx) = progress {
            let _ = tx.send(Progress::new(format!("安装原版 {mc_version}")));
        }
        launch::install_version(dl, paths, vanilla_entry, progress.clone()).await?;
    }
    Ok(())
}

/// 收尾:解析装好的 loader profile 的完整 `inheritsFrom` 链,确保它引用的库都到位。
pub async fn finalize(
    dl: &Downloader,
    paths: &GamePaths,
    id: &str,
    progress: Option<watch::Sender<Progress>>,
) -> Result<()> {
    let profile = launch::resolve_disk_profile(paths, id)?;
    let ctx = RuntimeContext::default();
    launch::ensure_files(dl, paths, &profile, &ctx, progress).await
}

/// forge/neoforge 共用:挑 Java(给定或自动探测)→ 跑官方 installer jar →
/// 校验生成的版本目录,返回新版本 id。
pub async fn install_via_jar(
    dl: &Downloader,
    paths: &GamePaths,
    installer_url: &str,
    java_path: Option<PathBuf>,
    progress: &Option<watch::Sender<Progress>>,
) -> Result<String> {
    let java = match java_path {
        Some(p) => p,
        None => any_java().await.ok_or(CoreError::JavaNotFound { major: 8 })?,
    };
    let id = run_installer(dl, paths, installer_url, &java, progress.clone()).await?;
    verify_installed(paths, &id)?;
    Ok(id)
}
