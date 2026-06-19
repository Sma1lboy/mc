//! Quilt loader installation via the Quilt meta API. Structurally identical to
//! Fabric: the meta server returns a ready Mojang-format profile json that
//! `inheritsFrom` the vanilla version.

use serde::Deserialize;
use tokio::sync::watch;

use mc_types::{ManifestVersion, Progress};

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::launch;
use crate::paths::{ensure_dir, GamePaths};
use crate::version::{RuntimeContext, VersionJson};

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
        .or_else(|| {
            // fall back to the newest of any kind
            None
        })
        .ok_or_else(|| CoreError::other(format!("Quilt 不支持 Minecraft {mc_version}")))
}

/// Install Quilt for `mc_version`, ensuring vanilla is present first. Returns the
/// launchable profile id.
pub async fn install_quilt(
    dl: &Downloader,
    paths: &GamePaths,
    mc_version: &str,
    vanilla_entry: &ManifestVersion,
    progress: Option<watch::Sender<Progress>>,
) -> Result<String> {
    if !paths.version_json(mc_version).exists() {
        if let Some(tx) = &progress {
            let _ = tx.send(Progress::new(format!("安装原版 {mc_version}")));
        }
        launch::install_version(dl, paths, vanilla_entry, progress.clone()).await?;
    }

    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("解析 Quilt loader 版本"));
    }
    let loader_version = match pick_loader_version(dl, mc_version).await {
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
    };

    let profile_url =
        format!("{QUILT_META}/versions/loader/{mc_version}/{loader_version}/profile/json");
    let raw = dl.get_text(&profile_url).await?;

    let vj = VersionJson::parse(&raw)
        .map_err(|e| CoreError::Parse { what: "quilt profile json".into(), source: e })?;
    let id = vj.id.clone();
    ensure_dir(&paths.version_dir(&id))?;
    crate::fs::write_atomic(&paths.version_json(&id), raw.as_bytes())?;

    if let Some(tx) = &progress {
        let _ = tx.send(Progress::new("下载 Quilt 依赖库"));
    }
    let profile = launch::resolve_disk_profile(paths, &id)?;
    let ctx = RuntimeContext::default();
    launch::ensure_files(dl, paths, &profile, &ctx, progress).await?;

    Ok(id)
}
