//! `resolve_mods` — resolve project ids into concrete, download-ready file
//! refs, walking required dependencies.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};


use crate::modplatform::dependency::{resolve_dependencies, ModRef};
use crate::modplatform::ProviderId;

use super::*;

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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UnresolvedRef {
    pub provider: String,
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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
pub async fn tool_resolve_mods(
    ctx: &ChatToolsCtx,
    args: ResolveModsArgs,
) -> Result<ResolveModsOutput, ChatToolError> {
    let roots: Vec<ModRef> = args.project_ids.iter().map(|s| parse_mod_ref(s)).collect();
    let already: HashSet<String> = args
        .already_installed
        .unwrap_or_default()
        .iter()
        .map(|s| normalize_installed_key(s))
        .collect();

    let resolution = resolve_dependencies(
        &ctx.registry,
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

