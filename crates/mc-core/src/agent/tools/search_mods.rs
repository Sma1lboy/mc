//! `search_mods` — search all registered providers for individual mods.


use serde::{Deserialize, Serialize};


use crate::modplatform::provider::{
    DedupCapPolicy, ProviderTargets, PROVIDER_FANOUT,
};
use crate::modplatform::{ResourceKind, SearchQuery};

use super::*;

/// Per-query and total caps for `search_mods`.
const MOD_SEARCH_PER_QUERY_CAP: usize = 8;
const MOD_SEARCH_TOTAL_CAP: usize = 12;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SearchModsArgs {
    /// English search keywords for the mod / feature to find.
    pub query: String,
    /// Target Minecraft version, e.g. "1.20.1".
    pub mc_version: String,
    /// Target loader, e.g. "fabric".
    pub loader: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ModHit {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub downloads: u64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SearchModsOutput {
    pub mods: Vec<ModHit>,
}

/// Search all registered providers for individual mods.
pub async fn tool_search_mods(
    ctx: &ChatToolsCtx,
    args: SearchModsArgs,
) -> Result<SearchModsOutput, ChatToolError> {
    let mut query = SearchQuery::new(args.query.trim(), ResourceKind::Mod);
    query.limit = MOD_SEARCH_PER_QUERY_CAP as u32;
    query.game_version = Some(args.mc_version.clone());
    query.loader = Some(args.loader.clone());

    let policy = DedupCapPolicy {
        providers: ProviderTargets::All,
        per_query_cap: Some(MOD_SEARCH_PER_QUERY_CAP),
        total_cap: MOD_SEARCH_TOTAL_CAP,
    };
    let matches = ctx
        .registry
        .search_concurrent(std::slice::from_ref(&query), PROVIDER_FANOUT, policy)
        .await?;

    let mods = matches
        .into_iter()
        .map(|m| ModHit {
            provider: provider_slug(m.provider).to_string(),
            project_id: m.hit.id,
            slug: m.hit.slug,
            title: m.hit.title,
            downloads: m.hit.downloads,
            description: m.hit.description,
        })
        .collect();
    Ok(SearchModsOutput { mods })
}

