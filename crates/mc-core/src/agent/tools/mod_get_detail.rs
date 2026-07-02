//! `mod_get_detail` — one mod's metadata + newest versions for a target.


use serde::{Deserialize, Serialize};



use super::*;

/// Newest versions surfaced per `mod_get_detail` call, to keep the tool result
/// bounded no matter how many versions a project has published.
const MOD_DETAIL_VERSION_CAP: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModGetDetailArgs {
    /// "modrinth" (default) or "curseforge".
    #[serde(default)]
    pub provider: Option<String>,
    /// Project id of the mod (from search_mods / inspect_base_modpack).
    pub project_id: String,
    /// Target Minecraft version to filter versions by, e.g. "1.20.1".
    #[serde(default)]
    pub minecraft_version: Option<String>,
    /// Target loader to filter versions by, e.g. "fabric".
    #[serde(default)]
    pub loader: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModDetailProject {
    pub title: String,
    pub slug: String,
    pub description: String,
    pub categories: Vec<String>,
    pub downloads: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModDetailVersion {
    pub version_id: String,
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub dependencies_count: usize,
    pub filename: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModGetDetailOutput {
    pub project: ModDetailProject,
    /// Newest first, capped so the payload stays bounded.
    pub versions: Vec<ModDetailVersion>,
}

/// Fetch one mod's metadata + available versions for a target.
pub async fn tool_mod_get_detail(
    ctx: &ChatToolsCtx,
    args: ModGetDetailArgs,
) -> Result<ModGetDetailOutput, ChatToolError> {
    let provider_id = provider_from_slug(args.provider.as_deref().unwrap_or("modrinth"));
    let provider = ctx.registry.get(provider_id).ok_or_else(|| {
        ChatToolError::new(format!(
            "provider {} is not registered",
            provider_slug(provider_id)
        ))
    })?;

    let project_id = args.project_id.trim().to_string();
    let hits = provider.get_projects(std::slice::from_ref(&project_id)).await?;
    let hit = hits
        .into_iter()
        .next()
        .ok_or_else(|| ChatToolError::new(format!("no project found for id {project_id}")))?;
    let project = ModDetailProject {
        title: hit.title,
        slug: hit.slug,
        description: hit.description,
        categories: hit.categories,
        downloads: hit.downloads,
    };

    let versions = provider
        .list_versions(
            &project_id,
            args.minecraft_version.as_deref(),
            args.loader.as_deref(),
        )
        .await?
        .into_iter()
        // Providers return newest first; the cap keeps the payload bounded.
        .take(MOD_DETAIL_VERSION_CAP)
        .map(|v| ModDetailVersion {
            version_id: v.id.clone(),
            version_number: v.version_number.clone(),
            game_versions: v.game_versions.clone(),
            loaders: v.loaders.clone(),
            dependencies_count: v.dependencies.len(),
            filename: v.primary_file().map(|f| f.filename.clone()),
        })
        .collect();

    Ok(ModGetDetailOutput { project, versions })
}

