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

pub(super) fn verify_written_mrpack(
    output_path: &Path,
    approved: &ApprovedModpackBuild,
) -> Result<()> {
    let file = std::fs::File::open(output_path).map_err(|e| CoreError::io(output_path, e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;
    let index: MrpackIndex = {
        let mut index = archive
            .by_name("modrinth.index.json")
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        serde_json::from_reader(&mut index).map_err(|source| CoreError::Parse {
            what: "written modrinth.index.json".to_string(),
            source,
        })?
    };
    verify_written_mrpack_index(&index, approved)?;
    verify_written_mrpack_overrides(&mut archive, &index)?;
    Ok(())
}

fn verify_written_mrpack_index(index: &MrpackIndex, approved: &ApprovedModpackBuild) -> Result<()> {
    if index.format_version != 1 {
        return Err(CoreError::other(format!(
            "mrpack verification failed: expected formatVersion 1, got {}",
            index.format_version
        )));
    }
    if index.game != "minecraft" {
        return Err(CoreError::other(format!(
            "mrpack verification failed: expected game minecraft, got {}",
            index.game
        )));
    }
    verify_written_mrpack_target(index, approved)?;

    let mut seen_paths = HashSet::new();
    for file in &index.files {
        if !mrpack_index_path_is_safe(&file.path) {
            return Err(CoreError::other(format!(
                "mrpack verification failed: unsafe file path {}",
                file.path
            )));
        }
        if !seen_paths.insert(file.path.clone()) {
            return Err(CoreError::other(format!(
                "mrpack verification failed: duplicate file path {}",
                file.path
            )));
        }
        if file.downloads.iter().all(|url| url.trim().is_empty()) {
            return Err(CoreError::other(format!(
                "mrpack verification failed: indexed file {} missing downloads",
                file.path
            )));
        }
        if file.hashes.sha512.trim().is_empty() {
            return Err(CoreError::other(format!(
                "mrpack verification failed: indexed file {} missing sha512",
                file.path
            )));
        }
        if file.env.is_none() {
            return Err(CoreError::other(format!(
                "mrpack verification failed: indexed file {} missing env",
                file.path
            )));
        }
    }
    Ok(())
}

fn verify_written_mrpack_target(
    index: &MrpackIndex,
    approved: &ApprovedModpackBuild,
) -> Result<()> {
    let expected_mc = optional_json_string(&approved.target, "minecraft_version");
    if let Some(expected_mc) = expected_mc {
        if index.dependencies.minecraft.as_deref() != Some(expected_mc.as_str()) {
            return Err(CoreError::other(format!(
                "mrpack verification failed: minecraft dependency does not match target {expected_mc}"
            )));
        }
    }

    let Some(expected_loader) = optional_json_string(&approved.target, "loader") else {
        return Ok(());
    };
    let expected_loader = expected_loader.to_ascii_lowercase();
    let loader_dependencies = [
        ("fabric", index.dependencies.fabric_loader.as_deref()),
        ("quilt", index.dependencies.quilt_loader.as_deref()),
        ("forge", index.dependencies.forge.as_deref()),
        ("neoforge", index.dependencies.neoforge.as_deref()),
    ];
    let Some((expected_key, expected_value)) = loader_dependencies
        .iter()
        .find(|(loader, _)| *loader == expected_loader.as_str())
    else {
        return Err(CoreError::other(format!(
            "mrpack verification failed: unsupported target loader {expected_loader}"
        )));
    };
    if expected_value.is_none() {
        return Err(CoreError::other(format!(
            "mrpack verification failed: missing loader dependency for target {expected_key}"
        )));
    }
    if let Some((unexpected, _)) = loader_dependencies
        .iter()
        .find(|(loader, value)| *loader != *expected_key && value.is_some())
    {
        return Err(CoreError::other(format!(
            "mrpack verification failed: unexpected loader dependency {unexpected} for target {expected_key}"
        )));
    }
    Ok(())
}

fn verify_written_mrpack_overrides(
    archive: &mut zip::ZipArchive<std::fs::File>,
    index: &MrpackIndex,
) -> Result<()> {
    let indexed_paths = index
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<HashSet<_>>();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        for root in ["overrides/", "client-overrides/", "server-overrides/"] {
            let Some(path) = name.strip_prefix(root) else {
                continue;
            };
            if !mrpack_index_path_is_safe(path) {
                return Err(CoreError::other(format!(
                    "mrpack verification failed: unsafe override path {name}"
                )));
            }
            if indexed_paths.contains(path) {
                return Err(CoreError::other(format!(
                    "mrpack verification failed: override path {path} conflicts with indexed file"
                )));
            }
        }
    }
    Ok(())
}

fn mrpack_index_path_is_safe(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    !normalized.is_empty()
        && !normalized.starts_with('/')
        && normalized
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
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
        ExecutionOutcomeKind::Verifying => {
            run.status = AgentStatus::Running;
            run.phase = AgentPhase::Verifying;
            run.pending_approval = None;
            run.tools.clear();
            run.execution = Some(AgentExecutionMetadata {
                status: AgentExecutionStatus::Running,
                manifest: Some(manifest),
                blocked: None,
            });
            run.push_message(
                AgentMessageKind::Tool,
                "execution artifact written; verifying",
            );
            run.push_trace("execution artifact written; entering verifying phase");
            Ok(run)
        }
        ExecutionOutcomeKind::Completed => {
            let completed_from = run.phase.clone();
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
            if completed_from == AgentPhase::Verifying {
                run.push_trace("verification completed; entering completed phase");
            } else {
                run.push_trace("execution completed");
            }
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
        "verifying" | "verify" => Ok(ExecutionOutcome {
            kind: ExecutionOutcomeKind::Verifying,
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
    approval.title = "Execution manifest is blocked; adjust customization".to_string();
    approval.message =
        format!("The executor was blocked while compiling the mrpack manifest: {reason}. Change the extra mods or return to base-pack selection.");
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
        description: Some(format!(
            "Current base pack is blocked during execution: {reason}"
        )),
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
        title: "Execution manifest is blocked; choose a base pack".to_string(),
        message: format!(
            "The executor was blocked while processing the base pack: {reason}. Search for another base pack, or keep the current base pack and retry."
        ),
        options,
        available_decisions: approval_decisions("Keep this base pack", "Search base packs again"),
        tools: vec![update_build_restrictions_tool_spec()],
        plan: Some(ModpackAgentPlan {
            objective: run.user_prompt.clone(),
            summary_markdown: format!("Base-pack execution is blocked: {reason}"),
            risks: vec!["Continuing with the current base pack may hit the same execution block again."
                .to_string()],
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
        warnings: vec![format!("Execution manifest is blocked: {reason}")],
        restrictions,
    };
    let mut approval = requirements_approval(&run.user_prompt, &output);
    approval.title = "Execution manifest is blocked; adjust requirements".to_string();
    approval.message =
        format!("The executor cannot continue with the current version/loader/requirements: {reason}. Change the requirements before continuing.");
    Ok(approval)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    /// Local one-shot-per-connection HTTP server (mirrors the CLI/download test
    /// helpers). Accepts connections in a loop so a transient retry inside the
    /// downloader never deadlocks the test, replying with the same response each
    /// time. Returns the base URL (`http://addr`).
    fn one_response_server(status: u16, content_type: &'static str, body: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf);
                let reason = match status {
                    200 => "OK",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "OK",
                };
                let headers = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{addr}")
    }

    fn valid_base_mrpack_bytes() -> Vec<u8> {
        let index = MrpackIndex {
            format_version: 1,
            game: "minecraft".to_string(),
            version_id: "base-1.0.0".to_string(),
            name: "Base Pack".to_string(),
            summary: None,
            dependencies: MrpackDependencies {
                minecraft: Some("1.20.1".to_string()),
                fabric_loader: Some("0.15.7".to_string()),
                ..Default::default()
            },
            files: Vec::new(),
        };
        let index_json = serde_json::to_vec(&index).unwrap();
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut output);
            let options = zip::write::SimpleFileOptions::default();
            writer.start_file("modrinth.index.json", options).unwrap();
            writer.write_all(&index_json).unwrap();
            writer.finish().unwrap();
        }
        output.into_inner()
    }

    fn approved_build_for_archive(archive_file: serde_json::Value) -> ApprovedModpackBuild {
        ApprovedModpackBuild {
            base_pack: serde_json::json!({ "provider": "modrinth", "title": "Base Pack" }),
            target: serde_json::json!({ "minecraft_version": "1.20.1", "loader": "fabric" }),
            extra_mods: Vec::new(),
            execution_recipe: Some(serde_json::json!({
                "schema_version": 1,
                "kind": "mrpack_from_base_modpack",
                "format": "mrpack",
                "base_pack_ref": {
                    "source_ref": { "archive_file": archive_file }
                },
                "extra_mod_refs": []
            })),
        }
    }

    fn temp_output_path(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mc-core-execution-{tag}-{}-{nanos}.mrpack",
            std::process::id()
        ))
    }

    // A transient base-archive HTTP 404 must NOT `?`-propagate out of the
    // executor (which would abort `advance()` before the retry machinery runs).
    // It must RETURN a manifest the driver classifies as Retry.
    #[tokio::test]
    async fn transient_base_archive_http_404_returns_retry_manifest() {
        let server = one_response_server(404, "application/octet-stream", b"missing".to_vec());
        let archive_file = serde_json::json!({
            "url": format!("{server}/base.mrpack"),
            "filename": "base.mrpack",
            "sha1": null,
            "sha512": null,
            "size": null,
            "primary": true,
        });
        let approved = approved_build_for_archive(archive_file);
        let output_path = temp_output_path("http404");

        let manifest = execute_mrpack_build_to_path(&approved, &output_path)
            .await
            .expect("a transient 404 must return a manifest, not propagate Err");

        assert_eq!(
            manifest.get("status").and_then(|v| v.as_str()),
            Some("retry")
        );
        assert_eq!(
            manifest.get("error_kind").and_then(|v| v.as_str()),
            Some("download_404")
        );
        assert!(manifest_is_retryable_external_error(&manifest));
        let outcome = classify_execution_outcome(&manifest).unwrap();
        assert!(matches!(outcome.kind, ExecutionOutcomeKind::Retry));
        assert!(!output_path.exists());
    }

    // A checksum mismatch on freshly-fetched base-archive bytes (a CDN serving
    // corrupt data) is a `CoreError::Checksum`. It must surface as a retryable
    // manifest, not abort the run.
    #[tokio::test]
    async fn transient_base_archive_checksum_mismatch_returns_retry_manifest() {
        let archive = valid_base_mrpack_bytes();
        let server = one_response_server(200, "application/octet-stream", archive);
        let archive_file = serde_json::json!({
            "url": format!("{server}/base.mrpack"),
            "filename": "base.mrpack",
            "sha1": null,
            // Wrong (but well-formed) sha512 -> CoreError::Checksum from verify.
            "sha512": "0".repeat(128),
            "size": null,
            "primary": true,
        });
        let approved = approved_build_for_archive(archive_file);
        let output_path = temp_output_path("checksum");

        let manifest = execute_mrpack_build_to_path(&approved, &output_path)
            .await
            .expect("a transient checksum mismatch must return a manifest, not propagate Err");

        assert_eq!(
            manifest.get("status").and_then(|v| v.as_str()),
            Some("retry")
        );
        assert_eq!(
            manifest.get("error_kind").and_then(|v| v.as_str()),
            Some("source_unavailable")
        );
        assert!(manifest_is_retryable_external_error(&manifest));
        let outcome = classify_execution_outcome(&manifest).unwrap();
        assert!(matches!(outcome.kind, ExecutionOutcomeKind::Retry));
        assert!(!output_path.exists());
    }

    // A structural failure (the fetched base archive is not a valid .mrpack) is
    // NOT transient: it must keep blocking back to base-pack selection, never
    // turn into a retry.
    #[tokio::test]
    async fn structural_base_archive_not_mrpack_blocks() {
        let server = one_response_server(200, "application/octet-stream", b"not a zip".to_vec());
        let archive_file = serde_json::json!({
            "url": format!("{server}/base.mrpack"),
            "filename": "base.mrpack",
            "sha1": null,
            "sha512": null,
            "size": null,
            "primary": true,
        });
        let approved = approved_build_for_archive(archive_file);
        let output_path = temp_output_path("structural");

        let manifest = execute_mrpack_build_to_path(&approved, &output_path)
            .await
            .expect("a structural base archive returns a blocked manifest");

        assert_eq!(
            manifest.get("status").and_then(|v| v.as_str()),
            Some("blocked")
        );
        assert_eq!(
            manifest.get("replan_phase").and_then(|v| v.as_str()),
            Some("choose_base_pack_approval")
        );
        assert!(!manifest_is_retryable_external_error(&manifest));
        let outcome = classify_execution_outcome(&manifest).unwrap();
        assert!(matches!(outcome.kind, ExecutionOutcomeKind::Blocked));
        assert!(!output_path.exists());
    }
}
