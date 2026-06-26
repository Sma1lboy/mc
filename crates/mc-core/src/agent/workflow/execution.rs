use std::collections::HashSet;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use crate::download::Downloader;
use crate::modpack::formats::mrpack::MrpackIndex;

use super::*;

#[derive(Debug, Clone)]
pub struct MrpackOverrideFile {
    pub archive_path: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct MrpackExecutionBuild {
    pub archive_bytes: Vec<u8>,
    pub manifest: serde_json::Value,
}

/// Build a new `.mrpack` from a downloaded base `.mrpack` plus the
/// human-approved execution metadata.
///
/// This is the deterministic core of the exec phase: it performs no network
/// calls and no filesystem writes. A caller can download/verify the base archive,
/// optionally download non-remote override files, then atomically write the
/// returned archive bytes.
pub fn build_mrpack_from_base_archive_bytes(
    approved: &ApprovedModpackBuild,
    base_archive_bytes: &[u8],
    override_files: &[MrpackOverrideFile],
) -> Result<MrpackExecutionBuild> {
    let base_index = read_base_mrpack_index(base_archive_bytes)?;
    let manifest = compile_mrpack_execution_metadata(approved, &base_index)?;
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

    copy_base_archive_entries(base_archive_bytes, &mut writer, options, &mut written)?;
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

pub async fn execute_mrpack_build_to_path(
    approved: &ApprovedModpackBuild,
    output_path: &Path,
) -> Result<serde_json::Value> {
    if let Some(provider) = optional_json_string(&approved.base_pack, "provider") {
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
    let base_archive_bytes = downloader.get_bytes(&url).await?;
    verify_version_file_bytes("base archive", &archive_file, &base_archive_bytes)?;

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
    let preflight = compile_mrpack_execution_metadata(approved, &base_index)?;
    if preflight.get("status").and_then(|v| v.as_str()) != Some("ready") {
        return Ok(preflight);
    }

    let override_files = download_extra_override_files(&downloader, &preflight).await?;
    let built =
        build_mrpack_from_base_archive_bytes(approved, &base_archive_bytes, &override_files)?;
    if built.manifest.get("status").and_then(|v| v.as_str()) != Some("completed") {
        return Ok(built.manifest);
    }

    crate::fs::write_atomic(output_path, &built.archive_bytes)?;
    verify_written_mrpack(output_path)?;

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

fn approved_base_archive_file(approved: &ApprovedModpackBuild) -> Result<VersionFile> {
    let recipe = approved
        .execution_recipe
        .as_ref()
        .ok_or_else(|| CoreError::other("approved build missing execution_recipe"))?;
    let recipe_file = recipe
        .get("base_pack_ref")
        .and_then(|v| v.get("source_ref"))
        .and_then(|v| v.get("archive_file"))
        .and_then(version_file_from_payload);
    let target_file = approved
        .target
        .get("base_primary_file")
        .and_then(version_file_from_payload);
    recipe_file
        .or(target_file)
        .ok_or_else(|| CoreError::other("approved build missing base archive file metadata"))
}

async fn download_extra_override_files(
    downloader: &Downloader,
    manifest: &serde_json::Value,
) -> Result<Vec<MrpackOverrideFile>> {
    let sources = manifest
        .get("extra_override_sources")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut out = Vec::new();
    for source in sources {
        let archive_path = source
            .get("archive_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::other("extra override source missing archive_path"))?;
        let Some(safe_path) = safe_archive_entry_name(archive_path) else {
            return Err(CoreError::Zip(format!(
                "unsafe extra override archive path: {archive_path}"
            )));
        };
        let file = source
            .get("source_file")
            .and_then(version_file_from_payload)
            .ok_or_else(|| CoreError::other("extra override source missing source_file"))?;
        let bytes = downloader.get_bytes(&file.url).await?;
        verify_version_file_bytes(&safe_path, &file, &bytes)?;
        out.push(MrpackOverrideFile {
            archive_path: safe_path,
            bytes,
        });
    }
    Ok(out)
}

fn verify_version_file_bytes(label: &str, file: &VersionFile, bytes: &[u8]) -> Result<()> {
    if let Some(size) = file.size {
        if bytes.len() as u64 != size {
            return Err(CoreError::Download {
                url: file.url.clone(),
                reason: format!(
                    "{label} size mismatch: expected {size}, got {}",
                    bytes.len()
                ),
            });
        }
    }
    if let Some(expected) = file.sha512.as_deref().filter(|s| !s.trim().is_empty()) {
        let actual = sha512_hex(bytes);
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(CoreError::Checksum {
                path: PathBuf::from(label),
                expected: expected.to_string(),
                actual,
            });
        }
    } else if let Some(expected) = file.sha1.as_deref().filter(|s| !s.trim().is_empty()) {
        let actual = sha1_hex(bytes);
        if !actual.eq_ignore_ascii_case(expected.trim()) {
            return Err(CoreError::Checksum {
                path: PathBuf::from(label),
                expected: expected.to_string(),
                actual,
            });
        }
    }
    Ok(())
}

fn sha512_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    let mut hasher = sha2::Sha512::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::Digest;
    let mut hasher = sha1::Sha1::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn verify_written_mrpack(output_path: &Path) -> Result<()> {
    let file = std::fs::File::open(output_path).map_err(|e| CoreError::io(output_path, e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;
    let mut index = archive
        .by_name("modrinth.index.json")
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    let _: MrpackIndex =
        serde_json::from_reader(&mut index).map_err(|source| CoreError::Parse {
            what: "written modrinth.index.json".to_string(),
            source,
        })?;
    Ok(())
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

fn read_base_mrpack_index(base_archive_bytes: &[u8]) -> Result<MrpackIndex> {
    let cursor = Cursor::new(base_archive_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| CoreError::Zip(e.to_string()))?;
    let mut index = archive
        .by_name("modrinth.index.json")
        .map_err(|_| CoreError::other("base archive missing modrinth.index.json"))?;
    if index.size() > MAX_BASE_MANIFEST_BYTES {
        return Err(CoreError::Zip(format!(
            "modrinth.index.json exceeds maximum size of {MAX_BASE_MANIFEST_BYTES} bytes"
        )));
    }
    let parsed = serde_json::from_reader(&mut index).map_err(|source| CoreError::Parse {
        what: "modrinth.index.json".to_string(),
        source,
    })?;
    Ok(parsed)
}

fn copy_base_archive_entries<W: Write + std::io::Seek>(
    base_archive_bytes: &[u8],
    writer: &mut zip::ZipWriter<W>,
    options: zip::write::SimpleFileOptions,
    written: &mut HashSet<String>,
) -> Result<()> {
    let cursor = Cursor::new(base_archive_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| CoreError::Zip(e.to_string()))?;
    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = safe_archive_entry_name(entry.name()) else {
            return Err(CoreError::Zip(format!(
                "unsafe base archive entry: {}",
                entry.name()
            )));
        };
        if name == "modrinth.index.json" || !written.insert(name.clone()) {
            continue;
        }
        writer
            .start_file(name.as_str(), options)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        std::io::copy(&mut entry, writer).map_err(|e| CoreError::Zip(e.to_string()))?;
    }
    Ok(())
}

fn write_extra_override_files<W: Write + std::io::Seek>(
    manifest: &serde_json::Value,
    override_files: &[MrpackOverrideFile],
    writer: &mut zip::ZipWriter<W>,
    options: zip::write::SimpleFileOptions,
    written: &mut HashSet<String>,
) -> Result<()> {
    let sources = manifest
        .get("extra_override_sources")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for source in sources {
        let archive_path = source
            .get("archive_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::other("extra override source missing archive_path"))?;
        let Some(safe_path) = safe_archive_entry_name(archive_path) else {
            return Err(CoreError::Zip(format!(
                "unsafe extra override archive path: {archive_path}"
            )));
        };
        let file = override_files
            .iter()
            .find(|f| f.archive_path == safe_path)
            .ok_or_else(|| {
                CoreError::other(format!("missing downloaded override bytes for {safe_path}"))
            })?;
        if !written.insert(safe_path.clone()) {
            continue;
        }
        writer
            .start_file(safe_path.as_str(), options)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        writer
            .write_all(&file.bytes)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
    }
    Ok(())
}

fn safe_archive_entry_name(name: &str) -> Option<String> {
    let normalized = name.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') {
        return None;
    }
    if normalized
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    Some(normalized)
}

fn set_json_field(value: &mut serde_json::Value, key: &str, next: serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(key.to_string(), next);
    }
}

/// Exec phase 0: compile the human-approved plan metadata into execution-owned
/// metadata after the base `.mrpack` has been downloaded and parsed.
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
                extra_remote_files.push(serde_json::json!({
                    "title": title,
                    "project_id": optional_json_string(&extra_ref, "project_id"),
                    "version_id": source_ref.get("version_id").cloned(),
                    "file": remote_file,
                }));
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

/// Consume execution phase output and either advance deterministic execution or
/// explicitly return to the right planning/HITL gate.
///
/// The caller must pass metadata produced by deterministic execution code, for
/// example [`compile_mrpack_execution_metadata`]. This function never asks the
/// model to reinterpret the executor result.
pub fn continue_after_execution_manifest_result(
    mut run: AgentRunSnapshot,
    manifest: serde_json::Value,
) -> Result<AgentRunSnapshot> {
    if !matches!(
        run.phase,
        AgentPhase::ExecutionReady | AgentPhase::Executing | AgentPhase::Verifying
    ) {
        return Err(CoreError::other(format!(
            "execution result cannot be applied while run is in phase {:?}",
            run.phase
        )));
    }

    let outcome = classify_execution_outcome(&manifest)?;

    match outcome.kind {
        ExecutionOutcomeKind::Ready => {
            run.status = AgentStatus::Running;
            run.phase = AgentPhase::Executing;
            run.pending_approval = None;
            run.tools = vec![build_mrpack_artifact_tool_spec()];
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Ready,
                manifest: Some(manifest),
                blocked: None,
            });
            run.push_message(
                AgentMessageKind::Tool,
                "exec.compile_execution_manifest produced a ready manifest",
            );
            run.push_trace("execution manifest ready; entering executing phase");
            Ok(run)
        }
        ExecutionOutcomeKind::Completed => {
            run.status = AgentStatus::Completed;
            run.phase = AgentPhase::Completed;
            run.pending_approval = None;
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Completed,
                manifest: Some(manifest),
                blocked: None,
            });
            run.push_message(AgentMessageKind::Tool, "execution completed");
            run.push_trace("execution completed");
            Ok(run)
        }
        ExecutionOutcomeKind::Blocked => {
            let replan_phase = outcome.replan_phase.clone().ok_or_else(|| {
                CoreError::other("blocked execution outcome missing replan_phase")
            })?;
            let reason = outcome
                .reason
                .clone()
                .unwrap_or_else(|| execution_block_reason(&manifest));
            let blocked = ExecutionBlocked {
                phase: run.phase.clone(),
                reason: reason.clone(),
                replan_phase: Some(replan_phase.clone()),
                details: manifest
                    .get("blocked")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            };
            let approval = execution_replan_approval(&run, &replan_phase, &manifest, &reason)?;
            run.status = AgentStatus::WaitingForUser;
            run.phase = replan_phase;
            run.pending_approval = Some(approval);
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Blocked,
                manifest: Some(manifest),
                blocked: Some(blocked),
            });
            run.push_message(
                AgentMessageKind::Tool,
                format!("exec.compile_execution_manifest blocked: {reason}"),
            );
            run.push_trace("execution manifest blocked; returned to HITL gate");
            Ok(run)
        }
        ExecutionOutcomeKind::Retry => {
            let reason = outcome
                .reason
                .clone()
                .unwrap_or_else(|| "execution should retry".to_string());
            run.status = AgentStatus::Running;
            run.pending_approval = None;
            run.tools = vec![build_mrpack_artifact_tool_spec()];
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Retry,
                manifest: Some(manifest),
                blocked: Some(ExecutionBlocked {
                    phase: run.phase.clone(),
                    reason: reason.clone(),
                    replan_phase: outcome.replan_phase.clone(),
                    details: serde_json::Value::Null,
                }),
            });
            run.push_message(
                AgentMessageKind::Tool,
                format!("execution external error is retryable: {reason}"),
            );
            run.push_trace("execution result classified as retryable external error");
            Ok(run)
        }
        ExecutionOutcomeKind::Failed => {
            let reason = outcome
                .reason
                .clone()
                .unwrap_or_else(|| execution_block_reason(&manifest));
            let failed_at = run.phase.clone();
            let details = manifest
                .get("failed")
                .or_else(|| manifest.get("error"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            run.status = AgentStatus::Failed;
            run.phase = AgentPhase::Failed;
            run.pending_approval = None;
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Failed,
                manifest: Some(manifest),
                blocked: Some(ExecutionBlocked {
                    phase: failed_at,
                    reason: reason.clone(),
                    replan_phase: outcome.replan_phase.clone(),
                    details,
                }),
            });
            run.push_message(
                AgentMessageKind::Tool,
                format!("execution failed: {reason}"),
            );
            run.push_trace("execution failed with retry gate metadata");
            Ok(run)
        }
    }
}

fn classify_execution_outcome(manifest: &serde_json::Value) -> Result<ExecutionOutcome> {
    let status = manifest
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let reason = Some(execution_block_reason(manifest));
    let replan_phase = execution_replan_phase(manifest).ok();

    match status.as_str() {
        "ready" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Ready,
            reason: None,
            replan_phase: None,
        }),
        "completed" | "complete" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Completed,
            reason: None,
            replan_phase: None,
        }),
        "blocked" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Blocked,
            reason,
            replan_phase,
        }),
        "retry" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Retry,
            reason,
            replan_phase,
        }),
        "failed" if manifest_is_retryable_external_error(manifest) => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Retry,
            reason,
            replan_phase,
        }),
        "failed" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Failed,
            reason,
            replan_phase,
        }),
        other => Err(CoreError::other(format!(
            "unsupported execution manifest status: {other:?}"
        ))),
    }
}

fn manifest_is_retryable_external_error(manifest: &serde_json::Value) -> bool {
    if manifest
        .get("retryable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return true;
    }
    let kind = manifest
        .get("error_kind")
        .or_else(|| manifest.get("kind"))
        .or_else(|| manifest.get("category"))
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        kind.as_str(),
        "download_404"
            | "download_timeout"
            | "source_timeout"
            | "source_unavailable"
            | "network"
            | "network_timeout"
    )
}

fn execution_replan_phase(manifest: &serde_json::Value) -> Result<AgentPhase> {
    let raw = manifest
        .get("replan_phase")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("confirm_customization_approval");
    match raw {
        "confirm_customization_approval" | "customization" | "extra_mods" => {
            Ok(AgentPhase::ConfirmCustomizationApproval)
        }
        "choose_base_pack_approval" | "base_pack_search" | "base_pack" => {
            Ok(AgentPhase::ChooseBasePackApproval)
        }
        "configure_requirements_approval" | "requirements" | "target" => {
            Ok(AgentPhase::ConfigureRequirementsApproval)
        }
        other => Err(CoreError::other(format!(
            "unsupported execution replan_phase: {other}"
        ))),
    }
}

fn execution_block_reason(manifest: &serde_json::Value) -> String {
    let blocked = manifest
        .get("blocked")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .take(3)
                .map(|item| {
                    let title = item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("execution item");
                    let reason = item
                        .get("reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("blocked");
                    format!("{title}: {reason}")
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .filter(|s| !s.is_empty());
    blocked.unwrap_or_else(|| {
        manifest
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("execution manifest blocked")
            .to_string()
    })
}

fn execution_replan_approval(
    run: &AgentRunSnapshot,
    replan_phase: &AgentPhase,
    manifest: &serde_json::Value,
    reason: &str,
) -> Result<ApprovalRequest> {
    match replan_phase {
        AgentPhase::ConfirmCustomizationApproval => {
            customization_execution_blocked_approval(run, manifest, reason)
        }
        AgentPhase::ChooseBasePackApproval | AgentPhase::BasePackSearch => {
            base_pack_execution_blocked_approval(run, manifest, reason)
        }
        AgentPhase::ConfigureRequirementsApproval => {
            requirements_execution_blocked_approval(run, reason)
        }
        other => Err(CoreError::other(format!(
            "cannot return execution block to phase {other:?}"
        ))),
    }
}

fn customization_execution_blocked_approval(
    run: &AgentRunSnapshot,
    manifest: &serde_json::Value,
    reason: &str,
) -> Result<ApprovalRequest> {
    let approved = run
        .approved_build
        .as_ref()
        .ok_or_else(|| CoreError::other("execution block has no approved build"))?;
    let base = selected_base_from_approved_build(approved)?;
    let target = target_compatibility_from_payload(&approved.target);
    let (plan, mut approval) = customization_approval(
        &run.user_prompt,
        &base,
        &target,
        approved.base_pack.clone(),
        approved.extra_mods.clone(),
    );
    approval.title = "执行清单受阻，需要调整定制方案".to_string();
    approval.message =
        format!("执行器在编译 mrpack 清单时受阻: {reason}。请修改补充 mods，或返回重选底包。");
    if let Some(option) = approval
        .options
        .iter_mut()
        .find(|o| o.id == "confirm:recommended_customization")
    {
        if let Some(payload) = option.payload.as_mut().and_then(|v| v.as_object_mut()) {
            payload.insert("execution_blocked".to_string(), manifest.clone());
            if let Some(recipe) = approved.execution_recipe.clone() {
                payload.insert("execution_recipe".to_string(), recipe);
            }
        }
    }
    approval.plan = Some(plan);
    Ok(approval)
}

fn base_pack_execution_blocked_approval(
    run: &AgentRunSnapshot,
    manifest: &serde_json::Value,
    reason: &str,
) -> Result<ApprovalRequest> {
    let approved = run
        .approved_build
        .as_ref()
        .ok_or_else(|| CoreError::other("execution block has no approved build"))?;
    let base = selected_base_from_approved_build(approved)?;
    let provider = provider_slug(base.provider);
    let options = vec![ApprovalOption {
        id: format!("{provider}:{}", base.project_id),
        label: base.title.clone(),
        description: Some(format!("当前底包执行受阻: {reason}")),
        payload: Some({
            let mut payload = approved.base_pack.clone();
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("execution_blocked".to_string(), manifest.clone());
            }
            payload
        }),
    }];
    Ok(ApprovalRequest {
        id: crate::agent::state::new_id("approval"),
        kind: ApprovalKind::ChooseBasePack,
        title: "执行清单受阻，需要重选底包".to_string(),
        message: format!(
            "执行器在处理底包时受阻: {reason}。可以重新搜索底包，或保留当前底包重试。"
        ),
        options,
        available_decisions: approval_decisions("保留当前底包", "重新搜索底包"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: Some(ModpackAgentPlan {
            objective: run.user_prompt.clone(),
            summary_markdown: format!("底包执行受阻: {reason}"),
            risks: vec!["继续使用当前底包可能再次触发相同执行阻塞。".to_string()],
            planned_actions: vec![PlannedAction {
                id: "replan-base-pack".to_string(),
                label: "User revises base pack after execution block".to_string(),
                tool: "approval_gate".to_string(),
                args: serde_json::json!({ "kind": "choose_base_pack", "execution_blocked": true }),
                requires_approval: true,
            }],
            migration_notes: vec![],
        }),
    })
}

fn requirements_execution_blocked_approval(
    run: &AgentRunSnapshot,
    reason: &str,
) -> Result<ApprovalRequest> {
    let restrictions = run.restrictions.clone().unwrap_or_default();
    let output = UpdateBuildRestrictionsOutput {
        missing_fields: missing_restriction_fields(&restrictions),
        warnings: vec![format!("执行清单受阻: {reason}")],
        restrictions,
    };
    let mut approval = requirements_approval(&run.user_prompt, &output);
    approval.title = "执行清单受阻，需要调整规格".to_string();
    approval.message =
        format!("执行器发现当前 version/loader/需求规格无法继续: {reason}。请修改规格后再继续。");
    Ok(approval)
}
