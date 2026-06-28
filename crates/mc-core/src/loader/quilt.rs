//! Quilt loader installation via the Quilt meta API. Structurally identical to
//! Fabric: the meta server returns a ready Mojang-format profile json that
//! `inheritsFrom` the vanilla version.

use serde::Deserialize;
use tokio::sync::watch;

use mc_types::{ManifestVersion, Progress};

use super::installer;
use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::paths::GamePaths;

const QUILT_META: &str = "https://meta.quiltmc.org/v3";

#[derive(Debug, Deserialize)]
struct LoaderEntry {
    loader: LoaderInfo,
}

#[derive(Debug, Deserialize)]
struct LoaderInfo {
    version: String,
}

async fn pick_loader_version(dl: &Downloader, mc_version: &str) -> Result<String> {
    let url = format!("{QUILT_META}/versions/loader/{mc_version}");
    let list: Vec<LoaderEntry> = dl.get_json(&url).await?;
    list.into_iter()
        .map(|e| e.loader.version)
        // Quilt lists newest first; skip beta when a stable exists.
        .find(|v| !v.contains("beta"))
        .ok_or_else(|| CoreError::other(format!("Quilt 不支持 Minecraft {mc_version}")))
}

/// Install Quilt for `mc_version`, ensuring vanilla is present first. Returns the
/// launchable profile id.
/// `loader_version`: pin a specific Quilt loader (e.g. a modpack's `quilt-loader`
/// dependency); `None`/empty picks the newest stable (then newest available).
pub async fn install_quilt(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    vanilla_entry: &ManifestVersion,
    loader_version: Option<&str>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    installer::ensure_vanilla(dl, paths, mc_version, vanilla_entry, &progress).await?;

    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("解析 Quilt loader 版本"));
    }
    let loader_version = match loader_version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(pinned) => pinned.to_string(),
        None => match pick_loader_version(dl, mc_version).await {
            Ok(v) => v,
            Err(_) => {
                // no stable: take the newest available
                let url = format!("{QUILT_META}/versions/loader/{mc_version}");
                let list: Vec<LoaderEntry> = dl.get_json(&url).await?;
                list.into_iter()
                    .next()
                    .map(|e| e.loader.version)
                    .ok_or_else(|| CoreError::other(format!("Quilt 不支持 Minecraft {mc_version}")))?
            }
        },
    };

    // fetch-profile → persist → finalize tail is identical to Fabric; one owner.
    installer::install_meta_profile(dl, paths, "Quilt", QUILT_META, mc_version, &loader_version, progress).await
}
