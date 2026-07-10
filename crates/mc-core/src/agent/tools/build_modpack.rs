//! `build_modpack` — deterministically assemble and verify a `.mrpack`.
//! The only tool that writes to disk; every file is re-resolved through the
//! provider so the model cannot fabricate urls or hashes.


use serde::{Deserialize, Serialize};

use mc_types::JsonValue;

use crate::agent::build::{
    execute_mrpack_build_to_path_with_registry, ApprovedModpackBuild,
};
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::{ProviderId, VersionFile};

use super::*;

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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BuildTarget {
    pub mc_version: String,
    pub loader: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BuildModpackOutput {
    pub status: String,
    pub output_path: Option<String>,
    pub output_size: Option<u64>,
    /// Full execution manifest for diagnostics (blocked reasons, counts, ...).
    /// Wire stays `serde_json::Value`; the specta override shapes the exported TS
    /// as [`JsonValue`] (specta can't inline recursive `serde_json::Value`).
    #[specta(type = JsonValue)]
    pub manifest: serde_json::Value,
}

/// Fetch the single concrete provider file for a `(project_id, version_id)`.
/// `build_modpack` re-resolves every file through this so urls/hashes are trusted,
/// not model-supplied.
async fn resolve_build_file(
    registry: &ProviderRegistry,
    provider: ProviderId,
    project_id: &str,
    version_id: &str,
) -> Result<VersionFile, ChatToolError> {
    let p = registry.get(provider).ok_or_else(|| {
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

/// Deterministically assemble and verify a `.mrpack` — the only tool that writes to
/// disk. Re-resolves every file through the provider so the model cannot fabricate
/// urls or hashes.
pub async fn tool_build_modpack(
    ctx: &ChatToolsCtx,
    args: BuildModpackArgs,
) -> Result<BuildModpackOutput, ChatToolError> {
    let validation =
        tool_validate_modpack_plan(ctx, ValidateModpackPlanArgs::from(&args)).await?;
    if validation.report.is_blocked() {
        return Ok(BuildModpackOutput {
            status: "blocked".to_string(),
            output_path: None,
            output_size: None,
            manifest: serde_json::json!({
                "status": "blocked",
                "compatibility": validation.report,
            }),
        });
    }

    // Base pack: re-resolve the archive file from the provider so its url and
    // hashes are trusted, not model-supplied.
    let (base_pack_json, base_pack_ref, recipe_kind) = match &args.base_pack {
        Some(bp) => {
            let archive = resolve_build_file(
                &ctx.registry,
                ProviderId::Modrinth,
                &bp.project_id,
                &bp.version_id,
            )
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
        let file =
            resolve_build_file(&ctx.registry, provider, &m.project_id, &m.version_id).await?;
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

    let output_path = ctx.output_dir.join(safe_output_filename(&args.output_filename));
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ChatToolError::new(format!("failed to create output directory: {e}")))?;
    }

    let manifest =
        execute_mrpack_build_to_path_with_registry(&approved, &output_path, &ctx.registry).await?;

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
