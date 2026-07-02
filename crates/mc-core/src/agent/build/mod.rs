//! Deterministic `.mrpack` executor + base-modlist parser.
//!
//! This is the trust boundary behind the `build_modpack` / `inspect_base_modpack`
//! tools: it compiles an [`ApprovedModpackBuild`] into an execution manifest,
//! re-verifies every file hash, and writes the final archive atomically. No
//! model-supplied url/hash is ever trusted — everything is echoed from provider
//! data or recomputed from downloaded bytes.

mod archive;
mod execute;
mod fetch;
mod manifest;
mod modlist;
mod scratch;
mod verify;

#[cfg(test)]
mod tests;

pub(crate) use execute::execute_mrpack_build_to_path_with_registry;
pub(crate) use modlist::parse_base_modlist;


use serde::{Deserialize, Serialize};

use crate::modpack::export::modrinth::host_in_whitelist;
use crate::modplatform::{ProjectSideSupport, VersionFile};

/// The structured plan the user approved in chat: base pack + target + extra
/// mods, plus the deterministic execution recipe compiled from tool results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedModpackBuild {
    pub base_pack: serde_json::Value,
    pub target: serde_json::Value,
    #[serde(default)]
    pub extra_mods: Vec<serde_json::Value>,
    #[serde(
        default,
        alias = "mrpack_plan",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution_recipe: Option<serde_json::Value>,
}

fn optional_json_string(value: &serde_json::Value, field: &str) -> Option<String> {
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToOwned::to_owned)
}

const BASE_ARCHIVE_FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);
const MAX_BASE_ARCHIVE_BYTES: usize = 128 * 1024 * 1024;

fn version_file_payload(file: &VersionFile) -> serde_json::Value {
    serde_json::json!({
        "url": file.url.clone(),
        "filename": file.filename.clone(),
        "sha1": file.sha1.clone(),
        "sha512": file.sha512.clone(),
        "size": file.size,
        "primary": file.primary,
        "client_side": file.client_side,
        "server_side": file.server_side,
    })
}

fn version_file_from_payload(value: &serde_json::Value) -> Option<VersionFile> {
    Some(VersionFile {
        url: optional_json_string(value, "url")?,
        filename: optional_json_string(value, "filename")?,
        sha1: optional_json_string(value, "sha1"),
        sha512: optional_json_string(value, "sha512"),
        size: value.get("size").and_then(|v| v.as_u64()),
        primary: value
            .get("primary")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        client_side: ProjectSideSupport::from_modrinth(
            value.get("client_side").and_then(|v| v.as_str()),
        ),
        server_side: ProjectSideSupport::from_modrinth(
            value.get("server_side").and_then(|v| v.as_str()),
        ),
    })
}

/// Sanitize a provider-supplied filename to a safe basename (no separators, no
/// dot segments), so an archive path can never escape `mods/`.
fn safe_provider_filename(raw: &str) -> Option<String> {
    let normalized = raw.trim().replace('\\', "/");
    let basename = normalized
        .rsplit('/')
        .find(|part| !part.trim().is_empty())?
        .trim();
    if basename == "." || basename == ".." {
        return None;
    }
    let sanitized = crate::fs::sanitize_filename(basename, '-');
    if sanitized.trim().is_empty()
        || sanitized == "."
        || sanitized == ".."
        || sanitized.contains('/')
        || sanitized.contains('\\')
    {
        return None;
    }
    Some(sanitized)
}

/// Shape a mod file as a `modrinth.index.json` files[] entry, or `None` when it
/// can't be indexed remotely (missing sha512 / non-whitelisted host).
fn mrpack_file_payload_with_filename(
    file: &VersionFile,
    safe_filename: &str,
) -> Option<serde_json::Value> {
    let sha512 = file.sha512.as_deref().filter(|s| !s.trim().is_empty())?;
    if file.url.trim().is_empty() || !host_in_whitelist(&file.url) {
        return None;
    }
    Some(serde_json::json!({
        "path": format!("mods/{safe_filename}"),
        "downloads": [file.url.clone()],
        "hashes": {
            "sha512": sha512,
            "sha1": file.sha1.clone(),
        },
        "fileSize": file.size,
        "env": {
            "client": file.client_side.as_mrpack_env(),
            "server": file.server_side.as_mrpack_env(),
        }
    }))
}

fn source_ref_payload(value: &serde_json::Value) -> Option<serde_json::Value> {
    value
        .get("source_ref")
        .or_else(|| value.get("execution_source"))
        .cloned()
}

fn set_json_field(value: &mut serde_json::Value, key: &str, next: serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(key.to_string(), next);
    }
}

