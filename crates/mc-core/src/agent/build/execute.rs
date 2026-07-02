//! The executor entry point: download/verify the base archive, compile the
//! execution manifest, assemble the output `.mrpack` zip, write + verify it.

use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use std::path::Path;

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::ProjectSideSupport;

use super::*;
use super::archive::{copy_base_archive_entries, read_base_mrpack_index, write_extra_override_files, MrpackOverrideFile};
use super::fetch::{approved_base_archive_file, download_extra_override_files, get_execution_bytes, infer_base_file_env_overrides, verify_version_file_bytes};
use super::manifest::compile_mrpack_execution_metadata;
use super::scratch::{scratch_base_archive_bytes, scratch_base_index};
use super::verify::verify_written_mrpack;

#[derive(Debug, Clone)]
pub struct MrpackExecutionBuild {
    pub archive_bytes: Vec<u8>,
    pub manifest: serde_json::Value,
}

/// Build a new `.mrpack` from a downloaded base `.mrpack` plus the approved
/// execution metadata. Deterministic: no network calls, no filesystem writes.
pub(super) fn build_mrpack_from_base_archive_bytes_with_env_overrides(
    approved: &ApprovedModpackBuild,
    base_archive_bytes: &[u8],
    override_files: &[MrpackOverrideFile],
    env_overrides: &HashMap<String, (ProjectSideSupport, ProjectSideSupport)>,
) -> Result<MrpackExecutionBuild> {
    let base_index = read_base_mrpack_index(base_archive_bytes)?;
    let manifest = compile_mrpack_execution_metadata(approved, &base_index, env_overrides)?;
    if manifest.get("status").and_then(|v| v.as_str()) != Some("ready") {
        return Ok(MrpackExecutionBuild {
            archive_bytes: Vec::new(),
            manifest,
        });
    }

    let output_index = manifest
        .get("output_index")
        .cloned()
        .ok_or_else(|| CoreError::other("ready execution manifest missing output_index"))?;
    let output_index_bytes =
        serde_json::to_vec_pretty(&output_index).map_err(|source| CoreError::Parse {
            what: "compiled modrinth.index.json".to_string(),
            source,
        })?;

    let mut output = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut written = HashSet::new();

    writer
        .start_file("modrinth.index.json", options)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    writer
        .write_all(&output_index_bytes)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    written.insert("modrinth.index.json".to_string());

    let indexed_paths = output_index
        .get("files")
        .and_then(|v| v.as_array())
        .map(|files| {
            files
                .iter()
                .filter_map(|f| f.get("path").and_then(|p| p.as_str()).map(str::to_string))
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    copy_base_archive_entries(
        base_archive_bytes,
        &mut writer,
        options,
        &mut written,
        &indexed_paths,
    )?;
    write_extra_override_files(
        &manifest,
        override_files,
        &mut writer,
        options,
        &mut written,
    )?;

    writer.finish().map_err(|e| CoreError::Zip(e.to_string()))?;

    let mut completed = manifest.clone();
    set_json_field(&mut completed, "status", serde_json::json!("completed"));
    set_json_field(
        &mut completed,
        "archive_bytes",
        serde_json::json!(output.get_ref().len()),
    );

    Ok(MrpackExecutionBuild {
        archive_bytes: output.into_inner(),
        manifest: completed,
    })
}

pub(crate) async fn execute_mrpack_build_to_path_with_registry(
    approved: &ApprovedModpackBuild,
    output_path: &Path,
    registry: &ProviderRegistry,
) -> Result<serde_json::Value> {
    if let Some(provider) = optional_json_string(&approved.base_pack, "provider") {
        if provider == "scratch" {
            return execute_scratch_mrpack_build_to_path(approved, output_path).await;
        }
        if provider != "modrinth" {
            return Ok(blocked_manifest(
                "choose_base_pack_approval",
                "base pack provider is not supported by the mrpack executor",
                serde_json::json!([{
                    "title": optional_json_string(&approved.base_pack, "title")
                        .unwrap_or_else(|| "base pack".to_string()),
                    "provider": provider,
                    "reason": "agent execution is currently limited to Modrinth .mrpack base packs",
                }]),
            ));
        }
    }

    let archive_file = approved_base_archive_file(approved)?;
    let url = archive_file.url.trim().to_string();
    if url.is_empty() {
        return Ok(blocked_manifest(
            "choose_base_pack_approval",
            "base archive is missing download url",
            serde_json::json!([{ "title": "base pack", "reason": "missing archive url" }]),
        ));
    }

    let downloader = Downloader::new(4)?;
    let base_archive_bytes = match retryable_external_or_propagate(
        get_execution_bytes(&downloader, &url, "base archive").await,
    ) {
        Ok(bytes) => bytes,
        Err(outcome) => return outcome,
    };
    if let Err(outcome) = retryable_external_or_propagate(verify_version_file_bytes(
        "base archive",
        &archive_file,
        &base_archive_bytes,
    )) {
        return outcome;
    }

    let base_index = match read_base_mrpack_index(&base_archive_bytes) {
        Ok(index) => index,
        Err(err) => {
            return Ok(blocked_manifest(
                "choose_base_pack_approval",
                "base archive is not a supported Modrinth .mrpack",
                serde_json::json!([{
                    "title": optional_json_string(&approved.base_pack, "title")
                        .unwrap_or_else(|| "base pack".to_string()),
                    "reason": format!("expected root modrinth.index.json: {err}"),
                }]),
            ));
        }
    };
    let env_overrides = infer_base_file_env_overrides(&base_index, registry).await;
    let preflight = compile_mrpack_execution_metadata(approved, &base_index, &env_overrides)?;
    if preflight.get("status").and_then(|v| v.as_str()) != Some("ready") {
        return Ok(preflight);
    }

    let override_files = match retryable_external_or_propagate(
        download_extra_override_files(&downloader, &preflight).await,
    ) {
        Ok(files) => files,
        Err(outcome) => return outcome,
    };
    let built = build_mrpack_from_base_archive_bytes_with_env_overrides(
        approved,
        &base_archive_bytes,
        &override_files,
        &env_overrides,
    )?;
    if built.manifest.get("status").and_then(|v| v.as_str()) != Some("completed") {
        return Ok(built.manifest);
    }

    crate::fs::write_atomic(output_path, &built.archive_bytes)?;
    verify_written_mrpack(output_path, approved)?;

    let mut manifest = built.manifest;
    set_json_field(
        &mut manifest,
        "output_path",
        serde_json::json!(output_path.to_string_lossy().to_string()),
    );
    set_json_field(
        &mut manifest,
        "output_size",
        serde_json::json!(built.archive_bytes.len()),
    );
    Ok(manifest)
}

async fn execute_scratch_mrpack_build_to_path(
    approved: &ApprovedModpackBuild,
    output_path: &Path,
) -> Result<serde_json::Value> {
    let base_index = scratch_base_index(approved);
    let base_archive_bytes = scratch_base_archive_bytes(&base_index)?;
    let downloader = Downloader::new(4)?;
    let preflight = compile_mrpack_execution_metadata(approved, &base_index, &HashMap::new())?;
    if preflight.get("status").and_then(|v| v.as_str()) != Some("ready") {
        return Ok(preflight);
    }

    let override_files = match retryable_external_or_propagate(
        download_extra_override_files(&downloader, &preflight).await,
    ) {
        Ok(files) => files,
        Err(outcome) => return outcome,
    };
    let built = build_mrpack_from_base_archive_bytes_with_env_overrides(
        approved,
        &base_archive_bytes,
        &override_files,
        &HashMap::new(),
    )?;
    if built.manifest.get("status").and_then(|v| v.as_str()) != Some("completed") {
        return Ok(built.manifest);
    }

    crate::fs::write_atomic(output_path, &built.archive_bytes)?;
    verify_written_mrpack(output_path, approved)?;

    let mut manifest = built.manifest;
    set_json_field(
        &mut manifest,
        "output_path",
        serde_json::json!(output_path.to_string_lossy().to_string()),
    );
    set_json_field(
        &mut manifest,
        "output_size",
        serde_json::json!(built.archive_bytes.len()),
    );
    Ok(manifest)
}

fn blocked_manifest(
    replan_phase: &str,
    reason: &str,
    blocked: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "status": "blocked",
        "format": "mrpack",
        "replan_phase": replan_phase,
        "reason": reason,
        "blocked": blocked,
    })
}

/// Map a transient EXTERNAL failure (flaky CDN response) into a retryable
/// execution manifest. A base-archive 404/timeout, a download size/stream
/// failure, or a checksum mismatch on freshly-fetched bytes are not bugs in the
/// approved plan: they should drive the driver's retry/backoff path rather than
/// `?`-propagate out of the executor and abort the whole run.
///
/// The returned `error_kind` is drawn from the allowlist that
/// `manifest_is_retryable_external_error` recognizes, so `classify_execution_outcome`
/// routes the manifest to `ExecutionOutcomeKind::Retry`. Structural errors
/// (`Zip`/`Parse`/`Io`/`Other`/…) return `None` so the caller propagates them
/// unchanged and a real bug never becomes an infinite retry.
fn transient_external_retry_manifest(err: &CoreError) -> Option<serde_json::Value> {
    let error_kind = match err {
        CoreError::Checksum { .. } => "source_unavailable",
        CoreError::Download { reason, .. } => {
            if reason.contains("timed out") {
                "download_timeout"
            } else {
                "source_unavailable"
            }
        }
        CoreError::Network(source) => {
            if source.status().map(|status| status.as_u16()) == Some(404) {
                "download_404"
            } else if source.is_timeout() {
                "download_timeout"
            } else {
                "network"
            }
        }
        _ => return None,
    };
    Some(serde_json::json!({
        "schema_version": 1,
        "status": "retry",
        "format": "mrpack",
        "reason": format!("transient external error while building mrpack: {err}"),
        "error_kind": error_kind,
        "retryable": true,
    }))
}

/// Funnel an executor network/checksum step's result into the right control
/// flow. On success returns the value; on a transient external error returns
/// `Err(Ok(retry_manifest))`; on a structural error returns `Err(Err(err))`.
/// Both `Err` arms are already shaped as the executor's return type, so a caller
/// can `return outcome` directly to either surface the retry manifest or
/// propagate the error.
fn retryable_external_or_propagate<T>(
    result: Result<T>,
) -> std::result::Result<T, Result<serde_json::Value>> {
    match result {
        Ok(value) => Ok(value),
        Err(err) => match transient_external_retry_manifest(&err) {
            Some(manifest) => Err(Ok(manifest)),
            None => Err(Err(err)),
        },
    }
}

