//! Network fetch + hash verification for execution inputs (base archive,
//! override files), and client/server env inference for base files.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::modpack::formats::mrpack::MrpackIndex;
use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::{HashAlgo, ProjectSideSupport, ProviderId, VersionFile};

use super::*;
use super::archive::{safe_archive_entry_name, MrpackOverrideFile};

pub(super) async fn infer_base_file_env_overrides(
    base_index: &MrpackIndex,
    registry: &ProviderRegistry,
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

pub(super) fn approved_base_archive_file(approved: &ApprovedModpackBuild) -> Result<VersionFile> {
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

pub(super) async fn download_extra_override_files(
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

pub(super) async fn get_execution_bytes(downloader: &Downloader, url: &str, label: &str) -> Result<Vec<u8>> {
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

pub(super) fn verify_version_file_bytes(label: &str, file: &VersionFile, bytes: &[u8]) -> Result<()> {
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

