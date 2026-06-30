use std::collections::HashSet;
use std::io::{Cursor, Read};
use std::time::Instant;

use crate::download::Downloader;
use crate::error::{CoreError, Result};
use crate::modpack::formats::curseforge::FlameManifest;
use crate::modpack::formats::mrpack::MrpackIndex;
use crate::modplatform::dependency::ModRef;
use crate::modplatform::ProviderId;

use super::*;

pub(super) async fn fetch_base_modlist_cache(
    run: &mut AgentRunSnapshot,
    base_pack_payload: &serde_json::Value,
) -> Result<BaseModlistCache> {
    let archive_file = base_pack_payload
        .get("source_ref")
        .and_then(|v| v.get("archive_file"))
        .ok_or_else(|| CoreError::other("base pack missing source_ref.archive_file"))?;
    let url = archive_file
        .get("url")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::other("base archive missing download url"))?;

    let started = Instant::now();
    let downloader = Downloader::new(2)?;
    let bytes = tokio::time::timeout(BASE_ARCHIVE_FETCH_TIMEOUT, downloader.get_bytes(url))
        .await
        .map_err(|_| CoreError::Download {
            url: url.to_string(),
            reason: "base archive fetch timed out".to_string(),
        })??;
    ensure_base_archive_size(url, bytes.len())?;
    run.push_tool_trace(AgentToolTrace {
        event: "customization planning fetched base archive once".into(),
        stage: AgentPhase::CustomizationPlanning,
        iteration: 0,
        tool: "fetch_base_archive".into(),
        input: serde_json::json!({ "url": url }),
        output: serde_json::json!({ "bytes": bytes.len() }),
        duration_ms: started.elapsed().as_millis(),
        status: "ok".into(),
    });

    let started = Instant::now();
    let cache = base_modlist_cache_from_archive_bytes(&bytes)?;
    run.push_tool_trace(AgentToolTrace {
        event: "customization planning parsed base modlist".into(),
        stage: AgentPhase::CustomizationPlanning,
        iteration: 0,
        tool: "parse_base_modlist".into(),
        input: serde_json::json!({ "archive_bytes": bytes.len() }),
        output: serde_json::json!({
            "source_format": cache.source_format.clone(),
            "mod_refs": mod_ref_payloads(&cache.refs),
        }),
        duration_ms: started.elapsed().as_millis(),
        status: "ok".into(),
    });

    Ok(cache)
}

pub(super) fn ensure_base_archive_size(url: &str, bytes_len: usize) -> Result<()> {
    if bytes_len > MAX_BASE_ARCHIVE_BYTES {
        return Err(CoreError::Download {
            url: url.to_string(),
            reason: format!("base archive exceeds maximum size of {MAX_BASE_ARCHIVE_BYTES} bytes"),
        });
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn parse_base_modlist(archive_bytes: &[u8]) -> Result<Vec<ModRef>> {
    parse_base_modlist_with_format(archive_bytes).map(|(_, refs)| refs)
}

pub(super) fn base_modlist_cache_from_archive_bytes(
    archive_bytes: &[u8],
) -> Result<BaseModlistCache> {
    let (source_format, refs) = parse_base_modlist_with_format(archive_bytes)?;
    Ok(BaseModlistCache {
        refs,
        source_format,
        fetch_count: 1,
    })
}

fn parse_base_modlist_with_format(archive_bytes: &[u8]) -> Result<(String, Vec<ModRef>)> {
    if let Some(index_bytes) = read_shallow_zip_entry(archive_bytes, "modrinth.index.json")? {
        let index: MrpackIndex =
            serde_json::from_slice(&index_bytes).map_err(|source| CoreError::Parse {
                what: "modrinth.index.json".to_string(),
                source,
            })?;
        return Ok(("modrinth".to_string(), mod_refs_from_mrpack_index(&index)));
    }
    if let Some(manifest_bytes) = read_shallow_zip_entry(archive_bytes, "manifest.json")? {
        let manifest: FlameManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|source| CoreError::Parse {
                what: "manifest.json".to_string(),
                source,
            })?;
        if !manifest.is_valid() {
            return Err(CoreError::other(
                "base archive manifest.json is not a CurseForge modpack manifest",
            ));
        }
        return Ok((
            "curseforge".to_string(),
            mod_refs_from_curseforge_manifest(&manifest),
        ));
    }
    Err(CoreError::other(
        "base archive missing modrinth.index.json or CurseForge manifest.json",
    ))
}

fn read_shallow_zip_entry(archive_bytes: &[u8], basename: &str) -> Result<Option<Vec<u8>>> {
    let cursor = Cursor::new(archive_bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| CoreError::Zip(e.to_string()))?;
    let mut selected: Option<(usize, usize)> = None;
    for idx in 0..zip.len() {
        let file = zip
            .by_index(idx)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        let name = file.name().replace('\\', "/");
        let is_match = name.rsplit('/').next().is_some_and(|tail| tail == basename);
        if !is_match {
            continue;
        }
        let depth = name.matches('/').count();
        if selected.is_none_or(|(_, best_depth)| depth < best_depth) {
            selected = Some((idx, depth));
        }
    }

    let Some((idx, _)) = selected else {
        return Ok(None);
    };
    let file = zip
        .by_index(idx)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    if file.size() > MAX_BASE_MANIFEST_BYTES {
        return Err(CoreError::Zip(format!(
            "{basename} exceeds maximum size of {MAX_BASE_MANIFEST_BYTES} bytes"
        )));
    }
    let mut bytes = Vec::new();
    let mut limited = file.take(MAX_BASE_MANIFEST_BYTES + 1);
    limited
        .read_to_end(&mut bytes)
        .map_err(|e| CoreError::other(format!("failed to read {basename} from archive: {e}")))?;
    if bytes.len() as u64 > MAX_BASE_MANIFEST_BYTES {
        return Err(CoreError::Zip(format!(
            "{basename} exceeds maximum size of {MAX_BASE_MANIFEST_BYTES} bytes"
        )));
    }
    Ok(Some(bytes))
}

fn mod_refs_from_mrpack_index(index: &MrpackIndex) -> Vec<ModRef> {
    let mut seen = HashSet::new();
    let mut refs = Vec::new();
    for file in &index.files {
        if !file.client_supported() {
            continue;
        }
        let Some(project_id) = file
            .downloads
            .iter()
            .find_map(|url| modrinth_project_id_from_url(url))
        else {
            continue;
        };
        let r = ModRef::new(ProviderId::Modrinth, project_id);
        if seen.insert(r.key()) {
            refs.push(r);
        }
    }
    refs
}

fn mod_refs_from_curseforge_manifest(manifest: &FlameManifest) -> Vec<ModRef> {
    let mut seen = HashSet::new();
    let mut refs = Vec::new();
    for file in &manifest.files {
        if !file.required {
            continue;
        }
        let r = ModRef::new(ProviderId::CurseForge, file.project_id.to_string());
        if seen.insert(r.key()) {
            refs.push(r);
        }
    }
    refs
}

fn modrinth_project_id_from_url(url: &str) -> Option<String> {
    let marker = "/data/";
    let (_, tail) = url.split_once(marker)?;
    let project_id = tail.split('/').next()?.trim();
    if project_id.is_empty() {
        None
    } else {
        Some(project_id.to_string())
    }
}

pub(super) fn mod_ref_payloads(refs: &[ModRef]) -> Vec<serde_json::Value> {
    refs.iter()
        .map(|r| {
            serde_json::json!({
                "provider": provider_slug(r.provider),
                "project_id": r.project_id.clone(),
            })
        })
        .collect()
}
