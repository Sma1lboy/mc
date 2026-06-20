//! Forge installation via the official installer jar.

use std::path::PathBuf;

use tokio::sync::watch;

use mc_types::{ManifestVersion, Progress};

use crate::download::Downloader;
use crate::error::Result;
use crate::paths::GamePaths;

use super::installer;

/// Build the Forge installer URL for `mc_version` + `forge_build`
/// (e.g. mc "1.20.1", build "47.2.0").
fn installer_url(mc_version: &str, forge_build: &str) -> String {
    let full = format!("{mc_version}-{forge_build}");
    format!(
        "https://maven.minecraftforge.net/net/minecraftforge/forge/{full}/forge-{full}-installer.jar"
    )
}

/// Install Forge `forge_build` for `mc_version`. Ensures vanilla is present,
/// runs the installer, and returns the launchable version id it produced.
///
/// `forge_build` is the Forge number only (e.g. "47.2.0"); pass an explicit
/// `java_path` or `None` to auto-detect.
pub async fn install_forge(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    forge_build: &str,
    vanilla_entry: &ManifestVersion,
    java_path: Option<PathBuf>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    installer::ensure_vanilla(dl, paths, mc_version, vanilla_entry, &progress).await?;

    let url = installer_url(mc_version, forge_build);
    let id = installer::install_via_jar(dl, paths, &url, java_path, &progress).await?;

    // Make sure any libraries the profile references are present.
    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("校验 Forge 文件"));
    }
    installer::finalize(dl, paths, &id, progress).await?;

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_installer_url() {
        assert_eq!(
            installer_url("1.20.1", "47.2.0"),
            "https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.2.0/forge-1.20.1-47.2.0-installer.jar"
        );
    }
}
