use super::*;

pub(in crate::agent::workflow) fn verify_written_mrpack(
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

pub(super) fn verify_written_mrpack_index(
    index: &MrpackIndex,
    approved: &ApprovedModpackBuild,
) -> Result<()> {
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

pub(super) fn verify_written_mrpack_target(
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

pub(super) fn verify_written_mrpack_overrides(
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

pub(super) fn mrpack_index_path_is_safe(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    !normalized.is_empty()
        && !normalized.starts_with('/')
        && normalized
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub(super) fn blocked_manifest(
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
pub(super) fn transient_external_retry_manifest(err: &CoreError) -> Option<serde_json::Value> {
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
pub(super) fn retryable_external_or_propagate<T>(
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

pub(super) fn set_json_field(value: &mut serde_json::Value, key: &str, next: serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(key.to_string(), next);
    }
}
