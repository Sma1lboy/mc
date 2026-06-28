//! Fabric loader installation via the Fabric meta API.
//!
//! Fabric is the cleanest loader to install: its meta server hands back a ready
//! made Mojang-format version json with `inheritsFrom` pointing at the vanilla
//! version, so we just fetch it, write it to disk, and let the normal profile
//! merge + `ensure_files` pipeline take over.

use serde::Deserialize;
use tokio::sync::watch;

use mc_types::{ManifestVersion, Progress};

use super::installer;
use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::paths::GamePaths;

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";

#[derive(Debug, Deserialize)]
struct LoaderEntry {
    loader: LoaderInfo,
}

#[derive(Debug, Deserialize)]
struct LoaderInfo {
    version: String,
    #[serde(default)]
    stable: bool,
}

/// Resolve the loader version to use: the newest stable one, or the newest of
/// any kind if none are marked stable.
async fn pick_loader_version(dl: &Downloader, mc_version: &str) -> Result<String> {
    let url = format!("{FABRIC_META}/versions/loader/{mc_version}");
    let list: Vec<LoaderEntry> = dl.get_json(&url).await?;
    if list.is_empty() {
        return Err(CoreError::other(format!("Fabric 不支持 Minecraft {mc_version}")));
    }
    let chosen = list
        .iter()
        .find(|e| e.loader.stable)
        .or_else(|| list.first())
        .map(|e| e.loader.version.clone())
        .unwrap();
    Ok(chosen)
}

/// Install Fabric for `mc_version`, ensuring the vanilla version is present
/// first. Returns the id of the created Fabric profile (the launchable id).
///
/// `vanilla_entry` is the manifest entry for `mc_version`; pass it so we can
/// install vanilla if it is missing without re-fetching the manifest here.
/// `loader_version`: pin a specific Fabric loader (e.g. a modpack's `fabric-loader`
/// dependency); `None`/empty picks the newest stable. Honoring the pin matters for
/// modpack import — the author chose that loader for compatibility.
pub async fn install_fabric(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    vanilla_entry: &ManifestVersion,
    loader_version: Option<&str>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    // 1. Ensure vanilla is installed (Fabric's profile inheritsFrom it).
    installer::ensure_vanilla(dl, paths, mc_version, vanilla_entry, &progress).await?;

    // 2. Resolve the loader version (Fabric-specific pick: newest stable, else newest).
    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("解析 Fabric loader 版本"));
    }
    let loader_version = match loader_version.map(str::trim).filter(|v| !v.is_empty()) {
        Some(pinned) => pinned.to_string(),
        None => pick_loader_version(dl, mc_version).await?,
    };

    // 3. The fetch-profile → persist → finalize tail is identical to Quilt; one owner.
    installer::install_meta_profile(dl, paths, "Fabric", FABRIC_META, mc_version, &loader_version, progress)
        .await
}
