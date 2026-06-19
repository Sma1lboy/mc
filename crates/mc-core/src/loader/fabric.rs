//! Fabric loader installation via the Fabric meta API.
//!
//! Fabric is the cleanest loader to install: its meta server hands back a ready
//! made Mojang-format version json with `inheritsFrom` pointing at the vanilla
//! version, so we just fetch it, write it to disk, and let the normal profile
//! merge + `ensure_files` pipeline take over.

use serde::Deserialize;
use tokio::sync::watch;

use mc_types::{ManifestVersion, Progress};

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::launch;
use crate::paths::{ensure_dir, GamePaths};
use crate::version::{RuntimeContext, VersionJson};

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
pub async fn install_fabric(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    vanilla_entry: &ManifestVersion,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    install_fabric_version(dl, paths, mc_version, vanilla_entry, None, progress).await
}

/// Install Fabric with an optional exact loader version. `.mrpack` imports pass
/// their declared `fabric-loader` dependency here; normal installs leave it as
/// `None` and use the newest stable loader.
pub async fn install_fabric_version(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    vanilla_entry: &ManifestVersion,
    loader_version: Option<&str>,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    // 1. Ensure vanilla is installed (Fabric's profile inheritsFrom it).
    if !paths.version_json(mc_version).exists() {
        if let Some(tx) = &progress {
            let _ = tx.send(Progress::new(format!("安装原版 {mc_version}")));
        }
        launch::install_version(dl, paths, vanilla_entry, progress.clone()).await?;
    }

    // 2. Resolve the loader version and fetch its profile json.
    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("解析 Fabric loader 版本"));
    }
    let loader_version = match loader_version.filter(|v| !v.is_empty()) {
        Some(v) => v.to_string(),
        None => pick_loader_version(dl, mc_version).await?,
    };
    let profile_url =
        format!("{FABRIC_META}/versions/loader/{mc_version}/{loader_version}/profile/json");
    let raw = dl.get_text(&profile_url).await?;

    // 3. Parse just enough to learn the profile id, then persist it verbatim.
    let vj = VersionJson::parse(&raw)
        .map_err(|e| CoreError::Parse { what: "fabric profile json".into(), source: e })?;
    let id = vj.id.clone();
    let dir = paths.version_dir(&id);
    ensure_dir(&dir)?;
    let json_path = paths.version_json(&id);
    crate::fs::write_atomic(&json_path, raw.as_bytes())?;

    // 4. Resolve the full chain and download Fabric's extra libraries.
    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("下载 Fabric 依赖库"));
    }
    let profile = launch::resolve_disk_profile(paths, &id)?;
    let ctx = RuntimeContext::default();
    launch::ensure_files(dl, paths, &profile, &ctx, progress).await?;

    Ok(id)
}
