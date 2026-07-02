//! Zip-level assembly of the output `.mrpack`: read the base index, copy the
//! preserved base entries, append override files, with archive-path safety
//! checks on everything that lands in the zip.

use std::collections::HashSet;
use std::io::{Cursor, Write};

use crate::error::{CoreError, Result};
use crate::modpack::formats::mrpack::MrpackIndex;

use super::modlist::MAX_BASE_MANIFEST_BYTES;

#[derive(Debug, Clone)]
pub struct MrpackOverrideFile {
    pub archive_path: String,
    pub bytes: Vec<u8>,
}

pub(super) fn read_base_mrpack_index(base_archive_bytes: &[u8]) -> Result<MrpackIndex> {
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

pub(super) fn copy_base_archive_entries<W: Write + std::io::Seek>(
    base_archive_bytes: &[u8],
    writer: &mut zip::ZipWriter<W>,
    options: zip::write::SimpleFileOptions,
    written: &mut HashSet<String>,
    indexed_paths: &HashSet<String>,
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
        if name == "modrinth.index.json"
            || base_archive_entry_conflicts_with_index(&name, indexed_paths)
            || !written.insert(name.clone())
        {
            continue;
        }
        writer
            .start_file(name.as_str(), options)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        std::io::copy(&mut entry, writer).map_err(|e| CoreError::Zip(e.to_string()))?;
    }
    Ok(())
}

/// Base modpack archives sometimes ship an override copy of a file that the
/// compiled index also manages remotely (`overrides/mods/foo.jar` vs an indexed
/// `mods/foo.jar`). Keeping both fails `verify_written_mrpack_overrides`, so
/// the indexed (remote, checksummed) copy wins and the base override is dropped.
fn base_archive_entry_conflicts_with_index(name: &str, indexed_paths: &HashSet<String>) -> bool {
    override_payload_path(name).is_some_and(|path| indexed_paths.contains(path))
}

fn override_payload_path(name: &str) -> Option<&str> {
    ["overrides/", "client-overrides/", "server-overrides/"]
        .into_iter()
        .find_map(|root| name.strip_prefix(root))
}

pub(super) fn write_extra_override_files<W: Write + std::io::Seek>(
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

pub(super) fn safe_archive_entry_name(name: &str) -> Option<String> {
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

