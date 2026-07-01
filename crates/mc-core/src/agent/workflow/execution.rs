use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use crate::download::Downloader;
use crate::loader::clean_loader_version;
use crate::modpack::formats::mrpack::{MrpackDependencies, MrpackIndex};
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::{HashAlgo, ProjectSideSupport, ProviderId};
use mc_types::LoaderKind;

use super::*;

mod archive_io;
mod manifest;

#[cfg(test)]
mod tests;

pub(super) use archive_io::verify_written_mrpack;
use archive_io::{
    blocked_manifest, copy_base_archive_entries, read_base_mrpack_index,
    retryable_external_or_propagate, safe_archive_entry_name, set_json_field,
    write_extra_override_files,
};
pub use manifest::continue_after_execution_manifest_result;
#[cfg(test)]
use manifest::{classify_execution_outcome, manifest_is_retryable_external_error};

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
    build_mrpack_from_base_archive_bytes_with_env_overrides(
        approved,
        base_archive_bytes,
        override_files,
        &HashMap::new(),
    )
}

fn build_mrpack_from_base_archive_bytes_with_env_overrides(
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
    let indexed_paths = output_index
        .get("files")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CoreError::other("ready execution manifest output_index missing files array")
        })?
        .iter()
        .filter_map(|file| file.get("path").and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect::<HashSet<_>>();

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

pub async fn execute_mrpack_build_to_path(
    approved: &ApprovedModpackBuild,
    output_path: &Path,
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
    let env_overrides = infer_base_file_env_overrides(&base_index).await;
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

    let mut manifest = built.manifest;
    set_json_field(&mut manifest, "status", serde_json::json!("verifying"));
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

    let mut manifest = built.manifest;
    set_json_field(&mut manifest, "status", serde_json::json!("verifying"));
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

fn scratch_base_index(approved: &ApprovedModpackBuild) -> MrpackIndex {
    let minecraft = optional_json_string(&approved.target, "minecraft_version");
    let loader = optional_json_string(&approved.target, "loader");
    let mut dependencies = MrpackDependencies {
        minecraft: minecraft.clone(),
        ..Default::default()
    };
    if let Some(loader) = loader.as_deref() {
        set_loader_dependency(
            &mut dependencies,
            loader,
            scratch_loader_dependency(loader, minecraft.as_deref(), &approved.target),
        );
    }
    MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: "agent-scratch".to_string(),
        name: optional_json_string(&approved.base_pack, "title")
            .unwrap_or_else(|| "Agent scratch modpack".to_string()),
        summary: Some("Generated from an empty base set by the local agent".to_string()),
        dependencies,
        files: Vec::new(),
    }
}

fn set_loader_dependency(
    dependencies: &mut MrpackDependencies,
    loader: &str,
    version: Option<String>,
) {
    match loader {
        "fabric" => dependencies.fabric_loader = version,
        "quilt" => dependencies.quilt_loader = version,
        "forge" => dependencies.forge = version,
        "neoforge" => dependencies.neoforge = version,
        _ => {}
    }
}

fn scratch_loader_dependency(
    loader: &str,
    mc_version: Option<&str>,
    target: &serde_json::Value,
) -> Option<String> {
    target_loader_dependency(loader, mc_version, target)
        .or_else(|| known_scratch_loader_dependency(loader, mc_version).map(str::to_string))
}

fn target_loader_dependency(
    loader: &str,
    mc_version: Option<&str>,
    target: &serde_json::Value,
) -> Option<String> {
    let loader = loader.trim().to_ascii_lowercase();
    let mc_version = mc_version.unwrap_or_default();
    let kind = loader_kind(&loader)?;
    ["loader_version", "version_id"]
        .into_iter()
        .filter_map(|field| optional_json_string(target, field))
        .map(|version| clean_loader_version(&version, kind, mc_version))
        .find(|version| concrete_loader_dependency(version))
}

fn loader_kind(loader: &str) -> Option<LoaderKind> {
    match loader {
        "fabric" => Some(LoaderKind::Fabric),
        "quilt" => Some(LoaderKind::Quilt),
        "forge" => Some(LoaderKind::Forge),
        "neoforge" => Some(LoaderKind::NeoForge),
        _ => None,
    }
}

fn concrete_loader_dependency(version: &str) -> bool {
    let trimmed = version.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("latest")
        && trimmed.chars().any(|c| c.is_ascii_digit())
}

fn known_scratch_loader_dependency(loader: &str, mc_version: Option<&str>) -> Option<&'static str> {
    match (loader.trim().to_ascii_lowercase().as_str(), mc_version?) {
        ("fabric", _) => Some("0.16.14"),
        ("quilt", _) => Some("0.26.4"),
        ("forge", "1.20.1") => Some("47.2.0"),
        ("forge", "1.20.2") => Some("48.1.0"),
        ("forge", "1.20.4") => Some("49.0.31"),
        ("neoforge", "1.20.1") => Some("47.1.106"),
        ("neoforge", "1.20.4") => Some("20.4.237"),
        _ => None,
    }
}

fn scratch_base_archive_bytes(index: &MrpackIndex) -> Result<Vec<u8>> {
    let index_bytes = serde_json::to_vec_pretty(index).map_err(|source| CoreError::Parse {
        what: "scratch modrinth.index.json".to_string(),
        source,
    })?;
    let mut output = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file("modrinth.index.json", options)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    writer
        .write_all(&index_bytes)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    writer.finish().map_err(|e| CoreError::Zip(e.to_string()))?;
    Ok(output.into_inner())
}

async fn infer_base_file_env_overrides(
    base_index: &MrpackIndex,
) -> HashMap<String, (ProjectSideSupport, ProjectSideSupport)> {
    let missing_env_files = base_index.files.iter().filter(|file| file.env.is_none());
    let mut path_to_project_id = HashMap::<String, String>::new();
    let mut hash_fallbacks = Vec::<(String, String)>::new();

    for file in missing_env_files {
        if let Some(project_id) = file
            .downloads
            .iter()
            .find_map(|url| modrinth_project_id_from_cdn_url(url))
        {
            path_to_project_id.insert(file.path.clone(), project_id);
        } else if !file.hashes.sha512.trim().is_empty() {
            hash_fallbacks.push((file.path.clone(), file.hashes.sha512.trim().to_string()));
        }
    }

    let registry = ProviderRegistry::with_defaults();
    let Some(provider) = registry.get(ProviderId::Modrinth) else {
        return HashMap::new();
    };

    if !hash_fallbacks.is_empty() {
        let hashes = hash_fallbacks
            .iter()
            .map(|(_, hash)| hash.clone())
            .collect::<Vec<_>>();
        if let Ok(resolved_files) = provider.resolve_by_hashes(HashAlgo::Sha512, &hashes).await {
            for ((path, _), resolved) in hash_fallbacks.iter().zip(resolved_files) {
                if path_to_project_id.contains_key(path) {
                    continue;
                }
                if let Some(resolved) = resolved {
                    if !resolved.project_id.trim().is_empty() {
                        path_to_project_id.insert(path.clone(), resolved.project_id);
                    }
                }
            }
        }
    }

    if path_to_project_id.is_empty() {
        return HashMap::new();
    }

    let mut seen = HashSet::new();
    let project_ids = path_to_project_id
        .values()
        .filter_map(|project_id| {
            if seen.insert(project_id.clone()) {
                Some(project_id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let projects = match provider.get_projects(&project_ids).await {
        Ok(projects) => projects,
        Err(_) => return HashMap::new(),
    };
    let project_sides = projects
        .into_iter()
        .map(|project| (project.id, (project.client_side, project.server_side)))
        .collect::<HashMap<_, _>>();

    path_to_project_id
        .into_iter()
        .filter_map(|(path, project_id)| {
            project_sides
                .get(&project_id)
                .copied()
                .map(|sides| (path, sides))
        })
        .collect()
}

fn modrinth_project_id_from_cdn_url(download_url: &str) -> Option<String> {
    let trimmed = download_url.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefix = if lower.starts_with("https://cdn.modrinth.com/data/") {
        "https://cdn.modrinth.com/data/"
    } else if lower.starts_with("http://cdn.modrinth.com/data/") {
        "http://cdn.modrinth.com/data/"
    } else {
        return None;
    };

    let mut segments = trimmed[prefix.len()..].split('/');
    let project_id = segments.next()?.trim();
    let versions_segment = segments.next()?.trim();
    if project_id.is_empty() || versions_segment != "versions" {
        return None;
    }
    Some(project_id.to_string())
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
        let bytes = get_execution_bytes(downloader, &file.url, &safe_path).await?;
        verify_version_file_bytes(&safe_path, &file, &bytes)?;
        out.push(MrpackOverrideFile {
            archive_path: safe_path,
            bytes,
        });
    }
    Ok(out)
}

async fn get_execution_bytes(downloader: &Downloader, url: &str, label: &str) -> Result<Vec<u8>> {
    match tokio::time::timeout(
        BASE_ARCHIVE_FETCH_TIMEOUT,
        downloader.get_bytes_capped(url, MAX_BASE_ARCHIVE_BYTES),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(CoreError::Download {
            url: url.to_string(),
            reason: format!(
                "{label} download timed out after {} seconds",
                BASE_ARCHIVE_FETCH_TIMEOUT.as_secs()
            ),
        }),
    }
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
