//! Resolve Minecraft item ids to local icon images for installed instances.
//!
//! This is deliberately best-effort: recipe cards can still render item labels
//! when a texture cannot be resolved.

use std::fs;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{base64_encode, sniff_image_mime, Instance};
use crate::error::{CoreError, IoResultExt, Result};
use crate::version::{InheritNode, VersionHead};

const ICON_MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct ItemIcon {
    pub item_id: String,
    pub data_url: String,
    pub source: String,
}

pub fn resolve_item_icon(inst: &Instance, item_id: &str) -> Result<Option<ItemIcon>> {
    let Some((namespace, path)) = parse_item_id(item_id) else {
        return Ok(None);
    };
    let query = ItemQuery { namespace, path };

    for root in local_asset_roots(inst) {
        if let Some(found) = resolve_from_dir(&root, &query)? {
            return Ok(Some(found.into_icon(item_id)));
        }
    }
    for root in directory_asset_roots(&inst.resourcepacks_dir())? {
        if let Some(found) = resolve_from_dir(&root, &query)? {
            return Ok(Some(found.into_icon(item_id)));
        }
    }
    for pack in archive_asset_roots(&inst.resourcepacks_dir())? {
        if let Some(found) = resolve_from_archive(&pack, &query)? {
            return Ok(Some(found.into_icon(item_id)));
        }
    }
    for jar in version_asset_roots(inst) {
        if let Some(found) = resolve_from_archive(&jar, &query)? {
            return Ok(Some(found.into_icon(item_id)));
        }
    }
    for jar in archive_asset_roots(&inst.mods_dir())? {
        if let Some(found) = resolve_from_archive(&jar, &query)? {
            return Ok(Some(found.into_icon(item_id)));
        }
    }

    Ok(None)
}

struct ItemQuery {
    namespace: String,
    path: String,
}

struct FoundIcon {
    bytes: Vec<u8>,
    source: String,
}

impl FoundIcon {
    fn into_icon(self, item_id: &str) -> ItemIcon {
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

fn parse_item_id(raw: &str) -> Option<(String, String)> {
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

fn local_asset_roots(inst: &Instance) -> Vec<PathBuf> {
    vec![inst.game_dir().join("kubejs"), inst.game_dir()]
}

fn archive_asset_roots(dir: &Path) -> Result<Vec<PathBuf>> {
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

fn version_asset_roots(inst: &Instance) -> Vec<PathBuf> {
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

fn directory_asset_roots(dir: &Path) -> Result<Vec<PathBuf>> {
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

fn resolve_from_dir(root: &Path, query: &ItemQuery) -> Result<Option<FoundIcon>> {
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

fn resolve_from_archive(path: &Path, query: &ItemQuery) -> Result<Option<FoundIcon>> {
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
    value
        .get("textures")
        .and_then(|v| v.get("layer0").or_else(|| v.get("all")))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(ToOwned::to_owned)
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

fn parse_model_ref(raw: &str, default_namespace: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains("..") || raw.starts_with('/') {
        return None;
    }
    let (namespace, path) = raw
        .split_once(':')
        .map(|(ns, path)| (ns, path))
        .unwrap_or((default_namespace, raw));
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
    let (namespace, path) = raw
        .split_once(':')
        .map(|(ns, path)| (ns, path))
        .unwrap_or((default_namespace, raw));
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

fn read_archive_text<R: Read + Seek>(
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::{Cursor, Write};
    use std::path::{Path, PathBuf};

    use zip::write::SimpleFileOptions;

    use super::resolve_item_icon;
    use crate::instance::{base64_encode, Instance};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nicon";

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mc-core-item-icon-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            let options = SimpleFileOptions::default();
            for (name, content) in entries {
                zip.start_file(*name, options).unwrap();
                zip.write_all(content).unwrap();
            }
            zip.finish().unwrap();
        }
        fs::write(path, bytes.into_inner()).unwrap();
    }

    #[test]
    fn resolves_item_icon_from_mod_jar_model_texture_reference() {
        let temp = TempRoot::new("mod-jar");
        let inst = Instance::new("pack", temp.path.clone());
        fs::create_dir_all(inst.mods_dir()).unwrap();
        write_zip(
            &inst.mods_dir().join("create.jar"),
            &[
                (
                    "assets/create/models/item/andesite_casing.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"create:item/andesite_casing"}}"#,
                ),
                ("assets/create/textures/item/andesite_casing.png", PNG),
            ],
        );

        let icon = resolve_item_icon(&inst, "create:andesite_casing")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "create:andesite_casing");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("create.jar"));
    }

    #[test]
    fn resolves_item_icon_from_kubejs_assets_before_mod_jar() {
        let temp = TempRoot::new("kubejs");
        let inst = Instance::new("pack", temp.path.clone());
        let texture = inst
            .game_dir()
            .join("kubejs/assets/create/textures/item/andesite_casing.png");
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        fs::write(&texture, PNG).unwrap();

        let icon = resolve_item_icon(&inst, "create:andesite_casing")
            .unwrap()
            .unwrap();

        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon
            .source
            .ends_with("kubejs/assets/create/textures/item/andesite_casing.png"));
    }

    #[test]
    fn resolves_item_icon_from_unpacked_resource_pack_directory() {
        let temp = TempRoot::new("resourcepack-dir");
        let inst = Instance::new("pack", temp.path.clone());
        let texture = inst
            .resourcepacks_dir()
            .join("pack/assets/create/textures/item/andesite_casing.png");
        fs::create_dir_all(texture.parent().unwrap()).unwrap();
        fs::write(&texture, PNG).unwrap();

        let icon = resolve_item_icon(&inst, "create:andesite_casing")
            .unwrap()
            .unwrap();

        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon
            .source
            .ends_with("pack/assets/create/textures/item/andesite_casing.png"));
    }

    #[test]
    fn resolves_vanilla_item_icon_from_inherited_version_jar() {
        let temp = TempRoot::new("version-jar");
        let inst = Instance::new("pack", temp.path.clone());
        let paths = inst.paths();
        fs::create_dir_all(paths.version_dir("pack")).unwrap();
        fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
        fs::write(
            paths.version_json("pack"),
            r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
        )
        .unwrap();
        fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
        write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/iron_nugget.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/iron_nugget"}}"#,
                ),
                ("assets/minecraft/textures/item/iron_nugget.png", PNG),
            ],
        );

        let icon = resolve_item_icon(&inst, "minecraft:iron_nugget")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "minecraft:iron_nugget");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("1.19.2.jar"));
    }

    #[test]
    fn resolves_block_item_icon_through_parent_model() {
        let temp = TempRoot::new("block-item-parent");
        let inst = Instance::new("pack", temp.path.clone());
        let paths = inst.paths();
        fs::create_dir_all(paths.version_dir("pack")).unwrap();
        fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
        fs::write(
            paths.version_json("pack"),
            r#"{"id":"pack","inheritsFrom":"1.19.2"}"#,
        )
        .unwrap();
        fs::write(paths.version_json("1.19.2"), r#"{"id":"1.19.2"}"#).unwrap();
        write_zip(
            &paths.version_jar("1.19.2"),
            &[
                (
                    "assets/minecraft/models/item/andesite.json",
                    br#"{"parent":"minecraft:block/andesite"}"#,
                ),
                (
                    "assets/minecraft/models/block/andesite.json",
                    br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/andesite"}}"#,
                ),
                ("assets/minecraft/textures/block/andesite.png", PNG),
            ],
        );

        let icon = resolve_item_icon(&inst, "minecraft:andesite")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "minecraft:andesite");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("1.19.2.jar"));
    }
}
