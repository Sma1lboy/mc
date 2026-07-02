//! `search_base_modpacks` — find existing modpacks usable as a base.


use serde::{Deserialize, Serialize};


use crate::modplatform::provider::{
    DedupCapPolicy, ProviderTargets, PROVIDER_FANOUT,
};
use crate::modplatform::{ProviderId, ResourceKind, SearchQuery, SortMethod};

use super::*;


/// Total base-pack candidates surfaced per `search_base_modpacks` call.
const BASE_SEARCH_TOTAL_CAP: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SearchBaseModpacksArgs {
    /// English search keywords describing the desired modpack.
    pub query: String,
    /// Target Minecraft version, e.g. "1.20.1". Omit to search all versions.
    #[serde(default)]
    pub mc_version: Option<String>,
    /// Target loader, e.g. "fabric" / "quilt" / "forge" / "neoforge".
    #[serde(default)]
    pub loader: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BaseModpackCandidate {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub author: String,
    pub downloads: u64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SearchBaseModpacksOutput {
    pub candidates: Vec<BaseModpackCandidate>,
}

/// Search Modrinth for modpacks usable as a base.
pub async fn tool_search_base_modpacks(
    ctx: &ChatToolsCtx,
    args: SearchBaseModpacksArgs,
) -> Result<SearchBaseModpacksOutput, ChatToolError> {
    let mut query = SearchQuery::new(args.query.trim(), ResourceKind::Modpack);
    query.limit = BASE_SEARCH_TOTAL_CAP as u32;
    query.sort = SortMethod::Relevance;
    query.game_version = args.mc_version.clone();
    query.loader = args.loader.clone();

    let policy = DedupCapPolicy {
        providers: ProviderTargets::Only(vec![ProviderId::Modrinth]),
        per_query_cap: None,
        total_cap: BASE_SEARCH_TOTAL_CAP,
    };
    let matches = ctx
        .registry
        .search_concurrent(std::slice::from_ref(&query), PROVIDER_FANOUT, policy)
        .await?;

    let candidates = matches
        .into_iter()
        .map(|m| BaseModpackCandidate {
            provider: provider_slug(m.provider).to_string(),
            project_id: m.hit.id,
            slug: m.hit.slug,
            title: m.hit.title,
            author: m.hit.author,
            downloads: m.hit.downloads,
            description: m.hit.description,
        })
        .collect();
    Ok(SearchBaseModpacksOutput { candidates })
}

