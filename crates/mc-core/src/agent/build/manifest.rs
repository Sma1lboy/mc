//! Exec phase 0: compile the human-approved plan metadata into an
//! execution-owned manifest (deterministic; no model / network / filesystem).

use std::collections::{HashMap, HashSet};

use crate::error::{CoreError, Result};
use crate::modpack::formats::mrpack::MrpackIndex;
use crate::modplatform::{ProjectSideSupport, VersionFile};

use super::*;

///
/// This function is deterministic and does not call the model, network, or
/// filesystem. A real executor should:
/// 1. download `approved.execution_recipe.base_pack_ref.source_ref.archive_file`;
/// 2. parse the base archive's `modrinth.index.json` into `base_index`;
/// 3. call this function;
/// 4. write a new `.mrpack` from `output_index`, preserved base overrides, and
///    `extra_override_sources`.
pub fn compile_mrpack_execution_metadata(
    approved: &ApprovedModpackBuild,
    base_index: &MrpackIndex,
    env_overrides: &HashMap<String, (ProjectSideSupport, ProjectSideSupport)>,
) -> Result<serde_json::Value> {
    let recipe = approved
        .execution_recipe
        .as_ref()
        .ok_or_else(|| CoreError::other("approved build missing execution_recipe"))?;
    let extra_refs = recipe
        .get("extra_mod_refs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| approved.extra_mods.clone());

    let mut output_index = serde_json::to_value(base_index).map_err(|source| CoreError::Parse {
        what: "base mrpack index".to_string(),
        source,
    })?;
    let files = output_index
        .get_mut("files")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| CoreError::other("base mrpack index serialized without files array"))?;
    for file in files.iter_mut() {
        if file.get("env").is_none() {
            let env = file
                .get("path")
                .and_then(|v| v.as_str())
                .and_then(|path| env_overrides.get(path))
                .copied()
                .map(|(client, server)| {
                    serde_json::json!({
                        "client": client.as_mrpack_env(),
                        "server": server.as_mrpack_env(),
                    })
                })
                .unwrap_or_else(|| {
                    serde_json::json!({
                        "client": "required",
                        "server": "required",
                    })
                });
            set_json_field(file, "env", env);
        }
    }

    let mut seen_paths = files
        .iter()
        .filter_map(|f| f.get("path").and_then(|v| v.as_str()).map(str::to_string))
        .collect::<HashSet<_>>();
    let mut extra_remote_files = Vec::new();
    let mut extra_override_sources = Vec::new();
    let mut deduped_extra_mods = Vec::new();
    let mut blocked = Vec::new();

    for extra_ref in extra_refs {
        let title = optional_json_string(&extra_ref, "title")
            .or_else(|| optional_json_string(&extra_ref, "slug"))
            .unwrap_or_else(|| "unknown extra mod".to_string());
        let Some(source_ref) = source_ref_payload(&extra_ref) else {
            blocked.push(serde_json::json!({
                "title": title,
                "reason": "missing source_ref",
                "replan_phase": "confirm_customization_approval",
            }));
            continue;
        };
        let Some(file) = source_ref.get("file").and_then(version_file_from_payload) else {
            blocked.push(serde_json::json!({
                "title": title,
                "reason": "missing resolved source file",
                "source_ref": source_ref,
                "replan_phase": "confirm_customization_approval",
            }));
            continue;
        };
        if file.filename.trim().is_empty() || file.url.trim().is_empty() {
            blocked.push(serde_json::json!({
                "title": title,
                "reason": "resolved source file has empty filename or url",
                "source_ref": source_ref,
                "replan_phase": "confirm_customization_approval",
            }));
            continue;
        }
        let Some(safe_filename) = safe_provider_filename(&file.filename) else {
            blocked.push(serde_json::json!({
                "title": title,
                "reason": "resolved source file has unsafe filename",
                "filename": file.filename,
                "source_ref": source_ref,
                "replan_phase": "confirm_customization_approval",
            }));
            continue;
        };

        if let Some(remote_file) = mrpack_file_payload_with_filename(&file, &safe_filename) {
            let Some(path) = remote_file.get("path").and_then(|v| v.as_str()) else {
                blocked.push(serde_json::json!({
                    "title": title,
                    "reason": "compiled remote file is missing path",
                    "source_ref": source_ref,
                    "replan_phase": "confirm_customization_approval",
                }));
                continue;
            };
            if seen_paths.insert(path.to_string()) {
                files.push(remote_file.clone());
                let mut remote_metadata = serde_json::json!({
                    "title": title,
                    "project_id": optional_json_string(&extra_ref, "project_id"),
                    "version_id": source_ref.get("version_id").cloned(),
                    "file": remote_file,
                    "project_side": {
                        "client": file.client_side,
                        "server": file.server_side,
                        "fallback": file.client_side.is_unknown() || file.server_side.is_unknown(),
                    },
                });
                if file.client_side.is_unknown() || file.server_side.is_unknown() {
                    set_json_field(
                        &mut remote_metadata,
                        "env_fallback",
                        serde_json::json!("unknown project side metadata mapped to optional"),
                    );
                }
                extra_remote_files.push(remote_metadata);
            } else {
                deduped_extra_mods.push(serde_json::json!({
                    "title": title,
                    "path": path,
                    "reason": "output path already exists in base modlist or earlier extra mod",
                }));
            }
            continue;
        }

        let install_path = format!("mods/{safe_filename}");
        let archive_path = format!("overrides/{install_path}");
        if seen_paths.insert(install_path.clone()) {
            if !version_file_has_verifiable_hash(&file) {
                blocked.push(serde_json::json!({
                    "title": title,
                    "reason": "override source has no verifiable hash",
                    "source_ref": source_ref,
                    "replan_phase": "confirm_customization_approval",
                }));
                continue;
            }
            extra_override_sources.push(serde_json::json!({
                "title": title,
                "project_id": optional_json_string(&extra_ref, "project_id"),
                "version_id": source_ref.get("version_id").cloned(),
                "install_path": install_path,
                "archive_path": archive_path,
                "source_file": version_file_payload(&file),
                "reason": "source is not eligible for mrpack remote files; executor must download and package it as an override",
            }));
        } else {
            deduped_extra_mods.push(serde_json::json!({
                "title": title,
                "path": install_path,
                "reason": "output path already exists in base modlist or earlier extra mod",
            }));
        }
    }

    let status = if blocked.is_empty() {
        "ready"
    } else {
        "blocked"
    };
    Ok(serde_json::json!({
        "schema_version": 1,
        "status": status,
        "format": "mrpack",
        "recipe_kind": recipe.get("kind").cloned(),
        "base": {
            "name": base_index.name.clone(),
            "version_id": base_index.version_id.clone(),
            "files_count": base_index.files.len(),
            "preserve_base_archive_overrides": true,
        },
        "output_index": output_index,
        "extra_remote_files": extra_remote_files,
        "extra_override_sources": extra_override_sources,
        "deduped_extra_mods": deduped_extra_mods,
        "blocked": blocked,
        "replan_phase": if status == "blocked" {
            Some("confirm_customization_approval")
        } else {
            None
        },
    }))
}

fn version_file_has_verifiable_hash(file: &VersionFile) -> bool {
    file.sha512
        .as_deref()
        .is_some_and(|hash| !hash.trim().is_empty())
        || file
            .sha1
            .as_deref()
            .is_some_and(|hash| !hash.trim().is_empty())
}

