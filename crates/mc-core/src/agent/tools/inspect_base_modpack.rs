//! `inspect_base_modpack` — download a base archive, list its mods and the
//! feature categories it already covers.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};


use crate::agent::build::parse_base_modlist;
use crate::download::Downloader;
use crate::modplatform::ProviderId;

use super::*;

/// Hard ceiling on a base modpack archive we will download to inspect it, so a
/// hostile/huge listing can't exhaust memory. 96 MiB comfortably covers real
/// `.mrpack` files (which ship only metadata + overrides, not the mod jars).
const MAX_BASE_ARCHIVE_BYTES: usize = 96 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InspectBaseModpackArgs {
    /// Modrinth project id of the base modpack (from search_base_modpacks).
    pub project_id: String,
    /// Target Minecraft version, used to pick the right pack version.
    #[serde(default)]
    pub mc_version: Option<String>,
    /// Target loader, used to pick the right pack version.
    #[serde(default)]
    pub loader: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InspectedMod {
    pub title: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InspectBaseModpackOutput {
    pub title: String,
    pub mc_version: Option<String>,
    pub loader: Option<String>,
    pub mod_count: usize,
    pub mods: Vec<InspectedMod>,
    /// Distinct mod categories present in the pack — a real, deterministic signal
    /// of which feature areas the base pack already covers. The model reads this
    /// to decide what still needs adding.
    pub covered_features: Vec<String>,
}

/// Inspect a base modpack archive and report its mods + covered feature areas.
pub async fn tool_inspect_base_modpack(
    ctx: &ChatToolsCtx,
    args: InspectBaseModpackArgs,
) -> Result<InspectBaseModpackOutput, ChatToolError> {
    let provider = ctx
        .registry
        .get(ProviderId::Modrinth)
        .ok_or_else(|| ChatToolError::new("Modrinth provider is not registered"))?;

    let mc = args.mc_version.as_deref();
    let loader = args.loader.as_deref();
    let versions = provider.list_versions(&args.project_id, mc, loader).await?;
    let version = versions
        .into_iter()
        .find(|v| v.primary_file().is_some())
        .ok_or_else(|| {
            ChatToolError::new(format!(
                "no downloadable version found for base pack {} (mc={:?}, loader={:?})",
                args.project_id, args.mc_version, args.loader
            ))
        })?;
    let archive = version
        .primary_file()
        .cloned()
        .ok_or_else(|| ChatToolError::new("selected base pack version has no primary file"))?;

    let downloader = Downloader::new(2)?;
    let bytes = downloader
        .get_bytes_capped(archive.url.trim(), MAX_BASE_ARCHIVE_BYTES)
        .await?;
    let refs = parse_base_modlist(&bytes)?;

    // Enrich each mod ref into a title + categories via the provider. Group
    // refs by provider so each backend gets one bulk call.
    let mut modrinth_ids = Vec::new();
    let mut curseforge_ids = Vec::new();
    for r in &refs {
        match r.provider {
            ProviderId::Modrinth => modrinth_ids.push(r.project_id.clone()),
            ProviderId::CurseForge => curseforge_ids.push(r.project_id.clone()),
        }
    }
    let mut mods = Vec::new();
    let mut categories = HashSet::new();
    for (pid, ids) in [
        (ProviderId::Modrinth, modrinth_ids),
        (ProviderId::CurseForge, curseforge_ids),
    ] {
        if ids.is_empty() {
            continue;
        }
        let Some(p) = ctx.registry.get(pid) else {
            continue;
        };
        let hits = p.get_projects(&ids).await?;
        for hit in hits {
            for c in &hit.categories {
                categories.insert(c.clone());
            }
            mods.push(InspectedMod {
                title: hit.title,
                categories: hit.categories,
            });
        }
    }
    mods.sort_by_key(|m| m.title.to_lowercase());
    let mut covered_features: Vec<String> = categories.into_iter().collect();
    covered_features.sort();

    let mc_version = mc
        .map(str::to_string)
        .or_else(|| version.game_versions.first().cloned());
    let loader = loader
        .map(str::to_string)
        .or_else(|| version.loaders.first().cloned());

    Ok(InspectBaseModpackOutput {
        title: version.name,
        mc_version,
        loader,
        mod_count: refs.len(),
        mods,
        covered_features,
    })
}

