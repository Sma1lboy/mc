use std::fs;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use super::ItemIcon;
use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::{base64_encode, sniff_image_mime, Instance};
use crate::version::{InheritNode, VersionHead};

const ICON_MAX_BYTES: usize = 512 * 1024;

pub(super) struct ItemQuery {
    pub(super) namespace: String,
    pub(super) path: String,
}
pub(super) struct FoundIcon {
    bytes: Vec<u8>,
    source: String,
}
impl FoundIcon {
    pub(super) fn into_icon(self, item_id: &str) -> ItemIcon {
        ItemIcon {
            item_id: item_id.to_string(),
            data_url: format!(
                "data:{};base64,{}",
                sniff_image_mime(&self.bytes),
                base64_encode(&self.bytes)
            ),
            source: self.source,
        }
    }
}

pub(super) fn parse_item_id(raw: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.starts_with('#') {
        return None;
    }
    let (namespace, path) = raw.split_once(':')?;
    let namespace = namespace.trim();
    let path = path.trim();
    if namespace.is_empty() || path.is_empty() || path.contains("..") || path.starts_with('/') {
        return None;
    }
    Some((namespace.to_string(), path.to_string()))
}
pub(super) fn local_asset_roots(inst: &Instance) -> Vec<PathBuf> {
    vec![inst.game_dir().join("kubejs"), inst.game_dir()]
}
pub(super) fn archive_asset_roots(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(CoreError::io(dir, e)),
    };
    for entry in entries {
        let entry = entry.with_path(dir)?;
        let path = entry.path();
        let lower = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if lower.ends_with(".jar") || lower.ends_with(".zip") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

pub(super) fn version_asset_roots(inst: &Instance) -> Vec<PathBuf> {
    let paths = inst.paths();
    let ids = crate::version::walk_inherits(inst.version_id(), |cur| {
        let parent = fs::read_to_string(paths.version_json(cur))
            .ok()
            .and_then(|raw| VersionHead::parse(&raw))
            .and_then(|head| head.inherits_from);
        Ok::<_, CoreError>(InheritNode {
            payload: cur.to_string(),
            parent,
        })
    })
    .unwrap_or_else(|_| vec![inst.version_id().to_string()]);

    ids.into_iter()
        .map(|id| paths.version_jar(&id))
        .filter(|path| path.is_file())
        .collect()
}

pub(super) fn directory_asset_roots(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(CoreError::io(dir, e)),
    };
    for entry in entries {
        let entry = entry.with_path(dir)?;
        let path = entry.path();
        if path.is_dir() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}
pub(super) fn resolve_from_dir(root: &Path, query: &ItemQuery) -> Result<Option<FoundIcon>> {
    let assets = root.join("assets").join(&query.namespace);
    if !assets.exists() {
        return Ok(None);
    }
    if let Some(texture_ref) = read_dir_model_texture(&assets, query)? {
        if let Some(found) = read_dir_texture(&assets, &query.namespace, &texture_ref)? {
            return Ok(Some(found));
        }
    }
    read_dir_texture(
        &assets,
        &query.namespace,
        &format!("{}:item/{}", query.namespace, query.path),
    )
}

fn read_dir_model_texture(assets: &Path, query: &ItemQuery) -> Result<Option<String>> {
    read_dir_model_texture_ref(
        assets,
        &query.namespace,
        &format!("{}:item/{}", query.namespace, query.path),
        0,
    )
}

fn read_dir_model_texture_ref(
    assets: &Path,
    default_namespace: &str,
    model_ref: &str,
    depth: usize,
) -> Result<Option<String>> {
    if depth > 16 {
        return Ok(None);
    }
    let Some((namespace, model_path)) = parse_model_ref(model_ref, default_namespace) else {
        return Ok(None);
    };
    if namespace != default_namespace {
        return Ok(None);
    }
    let model = assets.join("models").join(format!("{model_path}.json"));
    if !model.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&model).with_path(&model)?;
    if let Some(texture_ref) = model_texture_ref(&text) {
        return Ok(Some(texture_ref));
    }
    let Some(parent) = model_parent_ref(&text) else {
        return Ok(None);
    };
    read_dir_model_texture_ref(assets, &namespace, &parent, depth + 1)
}

fn read_dir_texture(
    assets: &Path,
    default_namespace: &str,
    texture_ref: &str,
) -> Result<Option<FoundIcon>> {
    let Some((namespace, texture_path)) = parse_texture_ref(texture_ref, default_namespace) else {
        return Ok(None);
    };
    if namespace != default_namespace {
        return Ok(None);
    }
    let path = assets.join("textures").join(format!("{texture_path}.png"));
    let Some(bytes) = read_icon_file(&path)? else {
        return Ok(None);
    };
    Ok(Some(FoundIcon {
        bytes,
        source: path.to_string_lossy().into_owned(),
    }))
}

pub(super) fn resolve_from_archive(path: &Path, query: &ItemQuery) -> Result<Option<FoundIcon>> {
    let file = fs::File::open(path).with_path(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;
    if let Some(texture_ref) = read_archive_model_texture(&mut archive, query)? {
        if let Some(bytes) = read_archive_texture(&mut archive, &query.namespace, &texture_ref)? {
            return Ok(Some(FoundIcon {
                bytes,
                source: path.to_string_lossy().into_owned(),
            }));
        }
    }
    let direct = format!("{}:item/{}", query.namespace, query.path);
    if let Some(bytes) = read_archive_texture(&mut archive, &query.namespace, &direct)? {
        return Ok(Some(FoundIcon {
            bytes,
            source: path.to_string_lossy().into_owned(),
        }));
    }
    Ok(None)
}

fn read_archive_model_texture<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    query: &ItemQuery,
) -> Result<Option<String>> {
    read_archive_model_texture_ref(
        archive,
        &query.namespace,
        &format!("{}:item/{}", query.namespace, query.path),
        0,
    )
}

fn read_archive_model_texture_ref<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    default_namespace: &str,
    model_ref: &str,
    depth: usize,
) -> Result<Option<String>> {
    if depth > 16 {
        return Ok(None);
    }
    let Some((namespace, model_path)) = parse_model_ref(model_ref, default_namespace) else {
        return Ok(None);
    };
    let name = format!("assets/{namespace}/models/{model_path}.json");
    let Some(text) = read_archive_text(archive, &name)? else {
        return Ok(None);
    };
    if let Some(texture_ref) = model_texture_ref(&text) {
        return Ok(Some(texture_ref));
    }
    let Some(parent) = model_parent_ref(&text) else {
        return Ok(None);
    };
    read_archive_model_texture_ref(archive, &namespace, &parent, depth + 1)
}

fn read_archive_texture<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    default_namespace: &str,
    texture_ref: &str,
) -> Result<Option<Vec<u8>>> {
    let Some((namespace, texture_path)) = parse_texture_ref(texture_ref, default_namespace) else {
        return Ok(None);
    };
    let name = format!("assets/{namespace}/textures/{texture_path}.png");
    read_archive_bytes(archive, &name)
}

fn model_texture_ref(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let textures = value.get("textures")?.as_object()?;
    for key in [
        "layer0", "all", "particle", "north", "south", "east", "west", "up", "down",
    ] {
        if let Some(texture) = resolve_model_texture_key(textures, key, 0) {
            return Some(texture);
        }
    }
    for value in textures.values() {
        let Some(raw) = value.as_str() else {
            continue;
        };
        if let Some(texture) = resolve_model_texture_value(textures, raw, 0) {
            return Some(texture);
        }
    }
    None
}

fn model_parent_ref(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    value
        .get("parent")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(ToOwned::to_owned)
}

fn resolve_model_texture_key(
    textures: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    depth: usize,
) -> Option<String> {
    let raw = textures.get(key)?.as_str()?;
    resolve_model_texture_value(textures, raw, depth)
}

fn resolve_model_texture_value(
    textures: &serde_json::Map<String, serde_json::Value>,
    raw: &str,
    depth: usize,
) -> Option<String> {
    if depth > 16 {
        return None;
    }
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(key) = raw.strip_prefix('#') {
        return resolve_model_texture_key(textures, key, depth + 1);
    }
    Some(raw.to_string())
}

fn parse_model_ref(raw: &str, default_namespace: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains("..") || raw.starts_with('/') {
        return None;
    }
    let (namespace, path) = raw.split_once(':').unwrap_or((default_namespace, raw));
    if namespace.is_empty() || path.is_empty() {
        return None;
    }
    Some((namespace.to_string(), path.to_string()))
}

fn parse_texture_ref(raw: &str, default_namespace: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains("..") || raw.starts_with('/') {
        return None;
    }
    let (namespace, path) = raw.split_once(':').unwrap_or((default_namespace, raw));
    if namespace.is_empty() || path.is_empty() {
        return None;
    }
    Some((namespace.to_string(), path.to_string()))
}

fn read_icon_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CoreError::io(path, e)),
    };
    if bytes.is_empty() || bytes.len() > ICON_MAX_BYTES {
        return Ok(None);
    }
    Ok(Some(bytes))
}

pub(super) fn read_archive_text<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<String>> {
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(CoreError::Zip(e.to_string())),
    };
    if entry.size() as usize > ICON_MAX_BYTES {
        return Ok(None);
    }
    let mut out = String::new();
    entry
        .read_to_string(&mut out)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    Ok(Some(out))
}

fn read_archive_bytes<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Option<Vec<u8>>> {
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(CoreError::Zip(e.to_string())),
    };
    if entry.size() as usize > ICON_MAX_BYTES {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry
        .read_to_end(&mut out)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    Ok((!out.is_empty()).then_some(out))
}
