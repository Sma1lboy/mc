//! The deterministic tools the chat agent can call.
//!
//! Each tool is a thin wrapper over an existing, tested `mc-core` primitive:
//! provider search, the dependency resolver, the base-modlist parser, and the
//! `.mrpack` executor. The tools take strictly-typed args (serde + schemars) and
//! return structured JSON built ONLY from real provider/resolver data — the model
//! can never fabricate project ids, version ids, urls, hashes, or filenames,
//! because those fields are always echoed straight from a provider call.
//!
//! `build_modpack` is the only tool that writes to disk, and it re-resolves every
//! file reference through the provider (`get_files_bulk`) rather than trusting
//! anything the model passed in.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use rig_core::completion::ToolDefinition;
use rig_core::tool::Tool;

use crate::agent::state::ApprovedModpackBuild;
use crate::agent::workflow::{
    base_modlist_cache_from_archive_bytes, execute_mrpack_build_to_path_with_registry,
};
use crate::download::Downloader;
use crate::modplatform::dependency::{resolve_dependencies, ModRef};
use crate::modplatform::provider::{
    DedupCapPolicy, ProviderRegistry, ProviderTargets, PROVIDER_FANOUT,
};
use crate::modplatform::{ProviderId, ResourceKind, SearchQuery, SortMethod, VersionFile};

/// Hard ceiling on a base modpack archive we will download to inspect it, so a
/// hostile/huge listing can't exhaust memory. 96 MiB comfortably covers real
/// `.mrpack` files (which ship only metadata + overrides, not the mod jars).
const MAX_BASE_ARCHIVE_BYTES: usize = 96 * 1024 * 1024;

/// Total base-pack candidates surfaced per `search_base_modpacks` call.
const BASE_SEARCH_TOTAL_CAP: usize = 8;
/// Per-query and total caps for `search_mods`.
const MOD_SEARCH_PER_QUERY_CAP: usize = 8;
const MOD_SEARCH_TOTAL_CAP: usize = 12;

/// Error surfaced by a chat tool. Wraps any `mc-core` failure as a string so the
/// model sees a readable message and can adapt (retry, ask the user, etc.).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ChatToolError(pub String);

impl ChatToolError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl From<crate::error::CoreError> for ChatToolError {
    fn from(err: crate::error::CoreError) -> Self {
        Self(err.to_string())
    }
}

/// Shared context injected into every tool: the provider registry to query and
/// the sandbox directory `build_modpack` writes finished `.mrpack` files into.
#[derive(Clone)]
pub struct ChatToolsCtx {
    /// Content-provider registry (Modrinth always; CurseForge when keyed).
    /// Injected, per the registry-injection convention, so tests use a fake.
    pub registry: Arc<ProviderRegistry>,
    /// Directory `build_modpack` writes into. The model supplies only a
    /// filename; the tool joins it here after sanitizing, so the model can never
    /// choose an arbitrary absolute path.
    pub output_dir: PathBuf,
}

impl ChatToolsCtx {
    pub fn new(registry: Arc<ProviderRegistry>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            registry,
            output_dir: output_dir.into(),
        }
    }
}

fn tool_parameters<T: JsonSchema>() -> serde_json::Value {
    let schema = schemars::SchemaGenerator::default().into_root_schema_for::<T>();
    serde_json::to_value(schema).unwrap_or_else(|_| serde_json::json!({ "type": "object" }))
}

fn provider_slug(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Modrinth => "modrinth",
        ProviderId::CurseForge => "curseforge",
    }
}

fn provider_from_slug(slug: &str) -> ProviderId {
    match slug.trim().to_ascii_lowercase().as_str() {
        "curseforge" => ProviderId::CurseForge,
        _ => ProviderId::Modrinth,
    }
}

/// Parse a `resolve_mods` root reference. Accepts a bare project id (defaults to
/// Modrinth) or a `"<provider>:<project_id>"` form so a CurseForge hit from
/// `search_mods` round-trips faithfully.
fn parse_mod_ref(raw: &str) -> ModRef {
    match raw.split_once(':') {
        Some((slug, id)) if slug == "modrinth" || slug == "curseforge" => {
            ModRef::new(provider_from_slug(slug), id.trim())
        }
        _ => ModRef::new(ProviderId::Modrinth, raw.trim()),
    }
}

/// Normalize an `already_installed` entry to the `<provider>:<project_id>` key
/// form that the dependency resolver dedupes on.
fn normalize_installed_key(raw: &str) -> String {
    parse_mod_ref(raw).key()
}

/// JSON shape read back by `compile_mrpack_execution_metadata`
/// (`version_file_from_payload`): url / filename / hashes / size / side support.
fn version_file_json(file: &VersionFile) -> serde_json::Value {
    serde_json::json!({
        "url": file.url,
        "filename": file.filename,
        "sha1": file.sha1,
        "sha512": file.sha512,
        "size": file.size,
        "primary": file.primary,
        "client_side": file.client_side,
        "server_side": file.server_side,
    })
}

// ===========================================================================
// search_base_modpacks
// ===========================================================================

#[derive(Debug, Clone, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize)]
pub struct BaseModpackCandidate {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub author: String,
    pub downloads: u64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchBaseModpacksOutput {
    pub candidates: Vec<BaseModpackCandidate>,
}

/// Search Modrinth for existing modpacks that could serve as a base.
#[derive(Clone)]
pub struct SearchBaseModpacksTool {
    pub registry: Arc<ProviderRegistry>,
}

impl Tool for SearchBaseModpacksTool {
    const NAME: &'static str = "search_base_modpacks";
    type Error = ChatToolError;
    type Args = SearchBaseModpacksArgs;
    type Output = SearchBaseModpacksOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search for existing Minecraft modpacks (on Modrinth) that could be used as a base pack. Returns real candidates with provider, project_id, slug, title, author, downloads, and description. Use English keywords.".to_string(),
            parameters: tool_parameters::<SearchBaseModpacksArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
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
        let matches = self
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
}

// ===========================================================================
// inspect_base_modpack
// ===========================================================================

#[derive(Debug, Clone, Deserialize, JsonSchema)]
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

#[derive(Debug, Clone, Serialize)]
pub struct InspectedMod {
    pub title: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
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

/// Download a base modpack archive and report the mods it bundles plus the
/// feature areas (categories) it already covers.
#[derive(Clone)]
pub struct InspectBaseModpackTool {
    pub registry: Arc<ProviderRegistry>,
}

impl Tool for InspectBaseModpackTool {
    const NAME: &'static str = "inspect_base_modpack";
    type Error = ChatToolError;
    type Args = InspectBaseModpackArgs;
    type Output = InspectBaseModpackOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Inspect a base modpack: download its archive, list the mods it already includes, and report the feature categories it covers. Use this before deciding which extra mods to add.".to_string(),
            parameters: tool_parameters::<InspectBaseModpackArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let provider = self
            .registry
            .get(ProviderId::Modrinth)
            .ok_or_else(|| ChatToolError::new("Modrinth provider is not registered"))?;

        let mc = args.mc_version.as_deref();
        let loader = args.loader.as_deref();
        let versions = provider
            .list_versions(&args.project_id, mc, loader)
            .await?;
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
        let cache = base_modlist_cache_from_archive_bytes(&bytes)?;

        // Enrich each mod ref into a title + categories via the provider. Group
        // refs by provider so each backend gets one bulk call.
        let mut modrinth_ids = Vec::new();
        let mut curseforge_ids = Vec::new();
        for r in &cache.refs {
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
            let Some(p) = self.registry.get(pid) else {
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
            mod_count: cache.refs.len(),
            mods,
            covered_features,
        })
    }
}

// ===========================================================================
// search_mods
// ===========================================================================

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SearchModsArgs {
    /// English search keywords for the mod / feature to find.
    pub query: String,
    /// Target Minecraft version, e.g. "1.20.1".
    pub mc_version: String,
    /// Target loader, e.g. "fabric".
    pub loader: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModHit {
    pub provider: String,
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub downloads: u64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchModsOutput {
    pub mods: Vec<ModHit>,
}

/// Search all registered providers for individual mods matching a query.
#[derive(Clone)]
pub struct SearchModsTool {
    pub registry: Arc<ProviderRegistry>,
}

impl Tool for SearchModsTool {
    const NAME: &'static str = "search_mods";
    type Error = ChatToolError;
    type Args = SearchModsArgs;
    type Output = SearchModsOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Search for individual Minecraft mods compatible with a Minecraft version + loader. Returns real candidates with provider, project_id, slug, title, downloads, and description. Use English keywords.".to_string(),
            parameters: tool_parameters::<SearchModsArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let mut query = SearchQuery::new(args.query.trim(), ResourceKind::Mod);
        query.limit = MOD_SEARCH_PER_QUERY_CAP as u32;
        query.game_version = Some(args.mc_version.clone());
        query.loader = Some(args.loader.clone());

        let policy = DedupCapPolicy {
            providers: ProviderTargets::All,
            per_query_cap: Some(MOD_SEARCH_PER_QUERY_CAP),
            total_cap: MOD_SEARCH_TOTAL_CAP,
        };
        let matches = self
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
}

// ===========================================================================
// resolve_mods
// ===========================================================================

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ResolveModsArgs {
    /// Project ids to resolve. Each is a bare id (Modrinth) or "<provider>:<id>".
    pub project_ids: Vec<String>,
    /// Target Minecraft version.
    pub mc_version: String,
    /// Target loader.
    pub loader: String,
    /// Project keys ("<provider>:<id>" or bare) already installed; treated as
    /// satisfied and not resolved again.
    #[serde(default)]
    pub already_installed: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedModRef {
    pub provider: String,
    pub project_id: String,
    pub version_id: String,
    pub filename: String,
    pub url: String,
    pub sha1: Option<String>,
    pub sha512: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnresolvedRef {
    pub provider: String,
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolveModsOutput {
    /// Concrete, download-ready file references (roots + required dependencies).
    /// These are the TRUSTED refs to pass to build_modpack.
    pub resolved: Vec<ResolvedModRef>,
    /// Project refs that have no version compatible with the target.
    pub unresolved: Vec<UnresolvedRef>,
    /// Project refs declared incompatible by a resolved version (conflicts).
    pub conflicts: Vec<UnresolvedRef>,
}

/// Resolve project ids into concrete files, walking required dependencies.
#[derive(Clone)]
pub struct ResolveModsTool {
    pub registry: Arc<ProviderRegistry>,
}

impl Tool for ResolveModsTool {
    const NAME: &'static str = "resolve_mods";
    type Error = ChatToolError;
    type Args = ResolveModsArgs;
    type Output = ResolveModsOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Resolve mod project ids into concrete, download-ready file references for a Minecraft version + loader, pulling in required dependencies. Returns resolved refs (with real version_id, url, hashes), plus anything unresolved or conflicting. The resolved refs are what you pass to build_modpack.".to_string(),
            parameters: tool_parameters::<ResolveModsArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let roots: Vec<ModRef> = args.project_ids.iter().map(|s| parse_mod_ref(s)).collect();
        let already: HashSet<String> = args
            .already_installed
            .unwrap_or_default()
            .iter()
            .map(|s| normalize_installed_key(s))
            .collect();

        let resolution = resolve_dependencies(
            &self.registry,
            &roots,
            args.mc_version.trim(),
            args.loader.trim(),
            &already,
        )
        .await?;

        let resolved = resolution
            .to_install
            .into_iter()
            .map(|r| ResolvedModRef {
                provider: provider_slug(r.provider).to_string(),
                project_id: r.project_id,
                version_id: r.version_id,
                filename: r.file.filename,
                url: r.file.url,
                sha1: r.file.sha1,
                sha512: r.file.sha512,
                size: r.file.size,
            })
            .collect();
        let unresolved = resolution
            .unresolved
            .into_iter()
            .map(|r| UnresolvedRef {
                provider: provider_slug(r.provider).to_string(),
                project_id: r.project_id,
            })
            .collect();
        let conflicts = resolution
            .incompatible
            .into_iter()
            .map(|r| UnresolvedRef {
                provider: provider_slug(r.provider).to_string(),
                project_id: r.project_id,
            })
            .collect();

        Ok(ResolveModsOutput {
            resolved,
            unresolved,
            conflicts,
        })
    }
}

// ===========================================================================
// build_modpack
// ===========================================================================

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BuildTarget {
    pub mc_version: String,
    pub loader: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BuildBasePack {
    /// Modrinth project id of the base pack.
    pub project_id: String,
    /// The exact base pack version id to build on (from inspect/search).
    pub version_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BuildModRef {
    /// "modrinth" (default) or "curseforge".
    #[serde(default)]
    pub provider: Option<String>,
    pub project_id: String,
    /// The resolved version id (from resolve_mods).
    pub version_id: String,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BuildModpackArgs {
    pub target: BuildTarget,
    /// The chosen base pack, or null to start from scratch (empty base).
    #[serde(default)]
    pub base_pack: Option<BuildBasePack>,
    /// Extra mods to add, as resolved refs from resolve_mods.
    #[serde(default)]
    pub extra_mods: Vec<BuildModRef>,
    /// Output file name (no path). ".mrpack" is appended if missing. The launcher
    /// decides the directory; the model only names the file.
    pub output_filename: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildModpackOutput {
    pub status: String,
    pub output_path: Option<String>,
    pub output_size: Option<u64>,
    /// Full execution manifest for diagnostics (blocked reasons, counts, ...).
    pub manifest: serde_json::Value,
}

/// Deterministically assemble and verify a `.mrpack`. THE ONLY tool that writes
/// to disk. Re-resolves every file through the provider so the model cannot
/// fabricate urls or hashes.
#[derive(Clone)]
pub struct BuildModpackTool {
    pub registry: Arc<ProviderRegistry>,
    pub output_dir: PathBuf,
}

impl BuildModpackTool {
    /// Fetch the single concrete provider file for a `(project_id, version_id)`.
    async fn resolve_file(
        &self,
        provider: ProviderId,
        project_id: &str,
        version_id: &str,
    ) -> Result<VersionFile, ChatToolError> {
        let p = self.registry.get(provider).ok_or_else(|| {
            ChatToolError::new(format!("provider {} is not registered", provider_slug(provider)))
        })?;
        let refs = [(project_id.to_string(), version_id.to_string())];
        let resolved = p.get_files_bulk(&refs).await?;
        resolved
            .into_iter()
            .next()
            .map(|r| r.file)
            .ok_or_else(|| {
                ChatToolError::new(format!(
                    "no file for {}:{} version {version_id}",
                    provider_slug(provider),
                    project_id
                ))
            })
    }
}

impl Tool for BuildModpackTool {
    const NAME: &'static str = "build_modpack";
    type Error = ChatToolError;
    type Args = BuildModpackArgs;
    type Output = BuildModpackOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Deterministically build and verify a .mrpack from a base pack (or from scratch) plus extra mods. THIS WRITES TO DISK — only call it after the user has explicitly confirmed the final plan.".to_string(),
            parameters: tool_parameters::<BuildModpackArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Base pack: re-resolve the archive file from the provider so its url and
        // hashes are trusted, not model-supplied.
        let (base_pack_json, base_pack_ref, recipe_kind) = match &args.base_pack {
            Some(bp) => {
                let archive = self
                    .resolve_file(ProviderId::Modrinth, &bp.project_id, &bp.version_id)
                    .await?;
                let slug = bp.slug.clone().unwrap_or_else(|| bp.project_id.clone());
                let title = bp.title.clone().unwrap_or_else(|| "base pack".to_string());
                let base_pack = serde_json::json!({
                    "provider": "modrinth",
                    "project_id": bp.project_id,
                    "slug": slug,
                    "title": title,
                });
                let base_ref = serde_json::json!({
                    "provider": "modrinth",
                    "project_id": bp.project_id,
                    "source_ref": { "archive_file": version_file_json(&archive) },
                });
                (base_pack, Some(base_ref), "mrpack_from_base_modpack")
            }
            None => (
                serde_json::json!({
                    "provider": "scratch",
                    "project_id": "scratch",
                    "slug": "scratch",
                    "title": "Start from scratch",
                }),
                None,
                "mrpack_from_scratch",
            ),
        };

        // Extra mods: re-resolve each file through its provider.
        let mut extra_mods = Vec::new();
        for m in &args.extra_mods {
            let provider = provider_from_slug(m.provider.as_deref().unwrap_or("modrinth"));
            let file = self
                .resolve_file(provider, &m.project_id, &m.version_id)
                .await?;
            let title = m.title.clone().unwrap_or_else(|| m.project_id.clone());
            extra_mods.push(serde_json::json!({
                "title": title,
                "project_id": m.project_id,
                "source_ref": {
                    "kind": "mod_file",
                    "provider": provider_slug(provider),
                    "project_id": m.project_id,
                    "version_id": m.version_id,
                    "file": version_file_json(&file),
                },
            }));
        }

        let execution_recipe = serde_json::json!({
            "schema_version": 1,
            "kind": recipe_kind,
            "format": "mrpack",
            "base_pack_ref": base_pack_ref,
            "extra_mod_refs": extra_mods.clone(),
        });
        let approved = ApprovedModpackBuild {
            base_pack: base_pack_json,
            target: serde_json::json!({
                "minecraft_version": args.target.mc_version,
                "loader": args.target.loader,
            }),
            extra_mods,
            execution_recipe: Some(execution_recipe),
        };

        let output_path = self.output_dir.join(safe_output_filename(&args.output_filename));
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ChatToolError::new(format!("failed to create output directory: {e}"))
            })?;
        }

        let manifest =
            execute_mrpack_build_to_path_with_registry(&approved, &output_path, &self.registry)
                .await?;

        let status = manifest
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let output_path_out = manifest
            .get("output_path")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let output_size = manifest.get("output_size").and_then(|v| v.as_u64());
        Ok(BuildModpackOutput {
            status,
            output_path: output_path_out,
            output_size,
            manifest,
        })
    }
}

/// Sanitize a model-supplied filename to a single safe basename ending in
/// `.mrpack`. Never yields a path separator, so the write stays inside
/// `output_dir`.
fn safe_output_filename(raw: &str) -> String {
    let base = raw
        .trim()
        .replace('\\', "/")
        .rsplit('/')
        .find(|s| !s.trim().is_empty())
        .unwrap_or("modpack")
        .to_string();
    let mut name = crate::fs::sanitize_filename(&base, '-');
    if name.trim().is_empty() || name == "." || name == ".." {
        name = "modpack".to_string();
    }
    if !name.to_lowercase().ends_with(".mrpack") {
        name.push_str(".mrpack");
    }
    name
}
