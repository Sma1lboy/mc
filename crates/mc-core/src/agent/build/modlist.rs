//! Base-modlist parsing: pull mod project refs out of a base modpack archive
//! (Modrinth `.mrpack` or CurseForge zip). Used by `inspect_base_modpack`.

use std::collections::HashSet;
use std::io::{Cursor, Read};

use crate::error::{CoreError, Result};
use crate::modpack::formats::curseforge::FlameManifest;
use crate::modpack::formats::mrpack::MrpackIndex;
use crate::modplatform::dependency::ModRef;
use crate::modplatform::ProviderId;


pub(super) const MAX_BASE_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;

/// Parse the client-side mod project refs out of a base modpack archive
/// (Modrinth `.mrpack` or CurseForge zip).
pub(crate) fn parse_base_modlist(archive_bytes: &[u8]) -> Result<Vec<ModRef>> {
    if let Some(index_bytes) = read_shallow_zip_entry(archive_bytes, "modrinth.index.json")? {
        let index: MrpackIndex =
            serde_json::from_slice(&index_bytes).map_err(|source| CoreError::Parse {
                what: "modrinth.index.json".to_string(),
                source,
            })?;
        return Ok(mod_refs_from_mrpack_index(&index));
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
        return Ok(mod_refs_from_curseforge_manifest(&manifest));
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

