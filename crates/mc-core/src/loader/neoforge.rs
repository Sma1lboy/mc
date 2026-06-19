//! NeoForge installation via the official installer jar.

use std::path::PathBuf;

use tokio::sync::watch;

use mc_types::{ManifestVersion, Progress};

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::launch;
use crate::paths::GamePaths;

use super::installer;

/// NeoForge installer URL for a NeoForge version (e.g. "20.4.237").
fn installer_url(neo_version: &str) -> String {
    format!(
        "https://maven.neoforged.net/releases/net/neoforged/neoforge/{neo_version}/neoforge-{neo_version}-installer.jar"
    )
}

/// Derive the Minecraft version a NeoForge build targets: "20.4.237" -> "1.20.4".
pub fn mc_version_for(neo_version: &str) -> Option<String> {
    let mut parts = neo_version.split('.');
    let minor = parts.next()?;
    let patch = parts.next()?;
    Some(format!("1.{minor}.{patch}"))
}

/// Install NeoForge `neo_version`. The target Minecraft version is derived from
/// the NeoForge version unless overridden.
pub async fn install_neoforge(
    dl: &Downloader,
    paths: &GamePaths,
    neo_version: &str,
    vanilla_entry: &ManifestVersion,
    java_path: Option<PathBuf>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    let mc_version = mc_version_for(neo_version)
        .ok_or_else(|| CoreError::other(format!("无法从 NeoForge 版本推断 MC 版本: {neo_version}")))?;

    if !paths.version_json(&mc_version).exists() {
        if let Some(tx) = &progress {
            let _ = tx.send(Progress::new(format!("安装原版 {mc_version}")));
        }
        launch::install_version(dl, paths, vanilla_entry, progress.clone()).await?;
    }

    let java = match java_path {
        Some(p) => p,
        None => installer::any_java()
            .await
            .ok_or(CoreError::JavaNotFound { major: 8 })?,
    };

    let url = installer_url(neo_version);
    let id = installer::run_installer(dl, paths, &url, &java, progress.clone()).await?;
    installer::verify_installed(paths, &id)?;

    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("校验 NeoForge 文件"));
    }
    let profile = launch::resolve_disk_profile(paths, &id)?;
    let ctx = crate::version::RuntimeContext::default();
    launch::ensure_files(dl, paths, &profile, &ctx, progress).await?;

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_mc_version() {
        assert_eq!(mc_version_for("20.4.237").as_deref(), Some("1.20.4"));
        assert_eq!(mc_version_for("21.0.0").as_deref(), Some("1.21.0"));
    }

    #[test]
    fn builds_url() {
        assert_eq!(
            installer_url("20.4.237"),
            "https://maven.neoforged.net/releases/net/neoforged/neoforge/20.4.237/neoforge-20.4.237-installer.jar"
        );
    }
}
