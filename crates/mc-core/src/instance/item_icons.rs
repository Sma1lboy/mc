//! Resolve Minecraft item ids to local icon images for installed instances.
//!
//! This is deliberately best-effort: recipe cards can still render item labels
//! when a texture cannot be resolved.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
    let item_id = item_id.trim();
    if item_id.starts_with('#') {
        return resolve_tag_icon(inst, item_id);
    }
    resolve_item_icon_id(inst, item_id, item_id)
}

fn resolve_item_icon_id(
    inst: &Instance,
    item_id: &str,
    display_item_id: &str,
) -> Result<Option<ItemIcon>> {
    let Some((namespace, path)) = parse_item_id(item_id) else {
        return Ok(None);
    };
    let query = ItemQuery { namespace, path };

    for root in local_asset_roots(inst) {
        if let Some(found) = resolve_from_dir(&root, &query)? {
            return Ok(Some(found.into_icon(display_item_id)));
        }
    }
    for root in directory_asset_roots(&inst.resourcepacks_dir())? {
        if let Some(found) = resolve_from_dir(&root, &query)? {
            return Ok(Some(found.into_icon(display_item_id)));
        }
    }
    for pack in archive_asset_roots(&inst.resourcepacks_dir())? {
        if let Some(found) = resolve_from_archive(&pack, &query)? {
            return Ok(Some(found.into_icon(display_item_id)));
        }
    }
    for jar in version_asset_roots(inst) {
        if let Some(found) = resolve_from_archive(&jar, &query)? {
            return Ok(Some(found.into_icon(display_item_id)));
        }
    }
    for jar in archive_asset_roots(&inst.mods_dir())? {
        if let Some(found) = resolve_from_archive(&jar, &query)? {
            return Ok(Some(found.into_icon(display_item_id)));
        }
    }

    Ok(None)
}

struct ItemQuery {
    namespace: String,
    path: String,
}

struct TagQuery {
    namespace: String,
    path: String,
}

struct FoundIcon {
    bytes: Vec<u8>,
    source: String,
}

#[derive(Debug, Deserialize)]
struct ItemTagFile {
    #[serde(default)]
    replace: bool,
    #[serde(default)]
    values: Vec<RawTagValue>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawTagValue {
    Id(String),
    Object {
        id: String,
        #[serde(rename = "required")]
        _required: Option<bool>,
    },
}

enum TagValue {
    Item(String),
    Tag(String),
}

enum FallbackItemCandidate {
    Id(String),
    Path(String),
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

fn parse_tag_id(raw: &str) -> Option<(String, String)> {
    let raw = raw.trim().strip_prefix('#')?;
    let (namespace, path) = raw.split_once(':')?;
    let namespace = namespace.trim();
    let path = path.trim();
    if namespace.is_empty() || path.is_empty() || path.contains("..") || path.starts_with('/') {
        return None;
    }
    Some((namespace.to_string(), path.to_string()))
}

fn normalize_tag_id(raw: &str) -> Option<String> {
    let (namespace, path) = parse_tag_id(raw)?;
    Some(format!("#{namespace}:{path}"))
}

fn local_asset_roots(inst: &Instance) -> Vec<PathBuf> {
    vec![inst.game_dir().join("kubejs"), inst.game_dir()]
}

fn local_data_roots(inst: &Instance) -> Vec<PathBuf> {
    vec![
        inst.game_dir().join("kubejs"),
        inst.datapacks_dir(),
        inst.game_dir(),
    ]
}

fn resolve_tag_icon(inst: &Instance, tag_id: &str) -> Result<Option<ItemIcon>> {
    let Some(tag_id) = normalize_tag_id(tag_id) else {
        return Ok(None);
    };
    let mut visited = HashSet::new();
    resolve_tag_icon_inner(inst, &tag_id, &tag_id, &mut visited, 0)
}

fn resolve_tag_icon_inner(
    inst: &Instance,
    tag_id: &str,
    display_item_id: &str,
    visited: &mut HashSet<String>,
    depth: usize,
) -> Result<Option<ItemIcon>> {
    if depth > 16 || !visited.insert(tag_id.to_string()) {
        return Ok(None);
    }

    if let Some(icon) = resolve_preferred_tag_icon(inst, tag_id, display_item_id)? {
        return Ok(Some(icon));
    }

    for value in read_item_tag_values(inst, tag_id)? {
        match value {
            TagValue::Item(item_id) => {
                if let Some(icon) = resolve_item_icon_id(inst, &item_id, display_item_id)? {
                    return Ok(Some(icon));
                }
            }
            TagValue::Tag(child_tag_id) => {
                if let Some(icon) = resolve_tag_icon_inner(
                    inst,
                    &child_tag_id,
                    display_item_id,
                    visited,
                    depth + 1,
                )? {
                    return Ok(Some(icon));
                }
            }
        }
    }

    if let Some(icon) = resolve_fallback_tag_icon(inst, tag_id, display_item_id)? {
        return Ok(Some(icon));
    }

    Ok(None)
}

fn resolve_preferred_tag_icon(
    inst: &Instance,
    tag_id: &str,
    display_item_id: &str,
) -> Result<Option<ItemIcon>> {
    let Some(item_id) = preferred_tag_representative(tag_id) else {
        return Ok(None);
    };
    resolve_item_icon_id(inst, item_id, display_item_id)
}

fn preferred_tag_representative(tag_id: &str) -> Option<&'static str> {
    match tag_id {
        "#minecraft:logs" => Some("minecraft:oak_log"),
        "#minecraft:planks" => Some("minecraft:oak_planks"),
        "#minecraft:saplings" => Some("minecraft:oak_sapling"),
        "#minecraft:stone_crafting_materials" => Some("minecraft:cobblestone"),
        "#minecraft:wooden_slabs" => Some("minecraft:oak_slab"),
        "#minecraft:wool" => Some("minecraft:white_wool"),
        _ => None,
    }
}

fn read_item_tag_values(inst: &Instance, tag_id: &str) -> Result<Vec<TagValue>> {
    let Some((namespace, path)) = parse_tag_id(tag_id) else {
        return Ok(Vec::new());
    };
    let query = TagQuery { namespace, path };
    let mut out = Vec::new();

    for root in local_data_roots(inst) {
        if let Some(tag) = read_tag_file_from_dir(&root, &query)? {
            let replace = tag.replace;
            append_tag_file_values(&mut out, tag);
            if replace {
                return Ok(out);
            }
        }
    }
    for root in directory_asset_roots(&inst.datapacks_dir())? {
        if let Some(tag) = read_tag_file_from_dir(&root, &query)? {
            let replace = tag.replace;
            append_tag_file_values(&mut out, tag);
            if replace {
                return Ok(out);
            }
        }
    }
    for pack in archive_asset_roots(&inst.datapacks_dir())? {
        if let Some(tag) = read_tag_file_from_archive(&pack, &query)? {
            let replace = tag.replace;
            append_tag_file_values(&mut out, tag);
            if replace {
                return Ok(out);
            }
        }
    }
    for jar in archive_asset_roots(&inst.mods_dir())? {
        if let Some(tag) = read_tag_file_from_archive(&jar, &query)? {
            let replace = tag.replace;
            append_tag_file_values(&mut out, tag);
            if replace {
                return Ok(out);
            }
        }
    }
    for jar in version_asset_roots(inst) {
        if let Some(tag) = read_tag_file_from_archive(&jar, &query)? {
            let replace = tag.replace;
            append_tag_file_values(&mut out, tag);
            if replace {
                return Ok(out);
            }
        }
    }

    Ok(out)
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

fn read_tag_file_from_dir(root: &Path, query: &TagQuery) -> Result<Option<ItemTagFile>> {
    let path = root
        .join("data")
        .join(&query.namespace)
        .join("tags")
        .join("items")
        .join(format!("{}.json", query.path));
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CoreError::io(path, e)),
    };
    Ok(parse_tag_file(&text))
}

fn read_tag_file_from_archive(path: &Path, query: &TagQuery) -> Result<Option<ItemTagFile>> {
    let file = fs::File::open(path).with_path(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;
    let name = format!("data/{}/tags/items/{}.json", query.namespace, query.path);
    let Some(text) = read_archive_text(&mut archive, &name)? else {
        return Ok(None);
    };
    Ok(parse_tag_file(&text))
}

fn parse_tag_file(text: &str) -> Option<ItemTagFile> {
    serde_json::from_str(text).ok()
}

fn append_tag_file_values(out: &mut Vec<TagValue>, tag: ItemTagFile) {
    for value in tag.values {
        if let Some(value) = tag_value_from_raw(value) {
            out.push(value);
        }
    }
}

fn tag_value_from_raw(value: RawTagValue) -> Option<TagValue> {
    let id = match value {
        RawTagValue::Id(id) => id,
        RawTagValue::Object { id, .. } => id,
    };
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    if id.starts_with('#') {
        return normalize_tag_id(id).map(TagValue::Tag);
    }
    parse_item_id(id).map(|(namespace, path)| TagValue::Item(format!("{namespace}:{path}")))
}

fn resolve_fallback_tag_icon(
    inst: &Instance,
    tag_id: &str,
    display_item_id: &str,
) -> Result<Option<ItemIcon>> {
    for candidate in fallback_item_candidates(tag_id) {
        match candidate {
            FallbackItemCandidate::Id(item_id) => {
                if let Some(icon) = resolve_item_icon_id(inst, &item_id, display_item_id)? {
                    return Ok(Some(icon));
                }
            }
            FallbackItemCandidate::Path(item_path) => {
                if let Some(icon) =
                    resolve_item_path_any_namespace(inst, &item_path, display_item_id)?
                {
                    return Ok(Some(icon));
                }
            }
        }
    }
    Ok(None)
}

fn fallback_item_candidates(tag_id: &str) -> Vec<FallbackItemCandidate> {
    let Some((namespace, path)) = parse_tag_id(tag_id) else {
        return Vec::new();
    };
    if !matches!(namespace.as_str(), "forge" | "c" | "fabric") {
        return Vec::new();
    }

    let mut out = Vec::new();
    match path.as_str() {
        "barrels/wooden" => push_candidate_id(&mut out, "minecraft:barrel"),
        "bones" => push_candidate_id(&mut out, "minecraft:bone"),
        "chests/wooden" => push_candidate_id(&mut out, "minecraft:chest"),
        "cobblestone" => push_candidate_id(&mut out, "minecraft:cobblestone"),
        "eggs" => push_candidate_id(&mut out, "minecraft:egg"),
        "ender_pearls" => push_candidate_id(&mut out, "minecraft:ender_pearl"),
        "glass" | "glass/colorless" => push_candidate_id(&mut out, "minecraft:glass"),
        "glass_panes/colorless" => push_candidate_id(&mut out, "minecraft:glass_pane"),
        "gunpowder" => push_candidate_id(&mut out, "minecraft:gunpowder"),
        "leather" => push_candidate_id(&mut out, "minecraft:leather"),
        "milk" => push_candidate_id(&mut out, "minecraft:milk_bucket"),
        "mushrooms" => push_candidate_id(&mut out, "minecraft:brown_mushroom"),
        "nether_stars" => push_candidate_id(&mut out, "minecraft:nether_star"),
        "netherrack" => push_candidate_id(&mut out, "minecraft:netherrack"),
        "ingots/nether_brick" | "nether_brick_ingots" => {
            push_candidate_id(&mut out, "minecraft:nether_brick")
        }
        "rods/blaze" | "blaze_rods" => push_candidate_id(&mut out, "minecraft:blaze_rod"),
        "rods/wooden" | "wooden_rods" => push_candidate_id(&mut out, "minecraft:stick"),
        "sand/colorless" => push_candidate_id(&mut out, "minecraft:sand"),
        "sand/red" => push_candidate_id(&mut out, "minecraft:red_sand"),
        "slimeballs" => push_candidate_id(&mut out, "minecraft:slime_ball"),
        "stone" => push_candidate_id(&mut out, "minecraft:stone"),
        "string" | "strings" => push_candidate_id(&mut out, "minecraft:string"),
        _ => {}
    }

    if let Some(material) = path.strip_prefix("ingots/") {
        push_material_candidates(&mut out, material, "ingot");
    } else if let Some(material) = path.strip_suffix("_ingots") {
        push_material_candidates(&mut out, material, "ingot");
    } else if let Some(material) = path.strip_prefix("nuggets/") {
        push_material_candidates(&mut out, material, "nugget");
    } else if let Some(material) = path.strip_suffix("_nuggets") {
        push_material_candidates(&mut out, material, "nugget");
    } else if let Some(material) = path.strip_prefix("dusts/") {
        push_dust_candidates(&mut out, material);
    } else if let Some(material) = path.strip_suffix("_dusts") {
        push_dust_candidates(&mut out, material);
    } else if let Some(material) = path.strip_prefix("gems/") {
        push_gem_candidates(&mut out, material);
    } else if let Some(material) = path.strip_suffix("_gems") {
        push_gem_candidates(&mut out, material);
    } else if let Some(item) = path.strip_prefix("fruits/") {
        push_candidate_path(&mut out, item);
        push_candidate_path(&mut out, &item.replace('_', ""));
    } else if let Some(material) = path.strip_prefix("plates/") {
        push_plate_candidates(&mut out, material);
    } else if let Some(material) = path.strip_suffix("_plates") {
        push_plate_candidates(&mut out, material);
    } else if let Some(material) = path.strip_prefix("storage_blocks/") {
        push_storage_block_candidates(&mut out, material);
    } else if let Some(material) = path.strip_suffix("_blocks") {
        push_storage_block_candidates(&mut out, material);
    }
    if !path.contains('/') {
        push_candidate_path(&mut out, &path);
    }

    out
}

fn push_dust_candidates(out: &mut Vec<FallbackItemCandidate>, material: &str) {
    match material {
        "redstone" => push_candidate_id(out, "minecraft:redstone"),
        "glowstone" => push_candidate_id(out, "minecraft:glowstone_dust"),
        _ => {}
    }
    push_material_candidates(out, material, "dust");
}

fn push_gem_candidates(out: &mut Vec<FallbackItemCandidate>, material: &str) {
    match material {
        "diamond" => push_candidate_id(out, "minecraft:diamond"),
        "emerald" => push_candidate_id(out, "minecraft:emerald"),
        "lapis" | "lapis_lazuli" => push_candidate_id(out, "minecraft:lapis_lazuli"),
        "nether_quartz" | "quartz" => push_candidate_id(out, "minecraft:quartz"),
        _ => {}
    }
    push_candidate_path(out, material);
    push_material_candidates(out, material, "gem");
}

fn push_plate_candidates(out: &mut Vec<FallbackItemCandidate>, material: &str) {
    push_material_candidates(out, material, "plate");
    if material == "gold" {
        push_candidate_path(out, "golden_sheet");
    }
    push_candidate_path(out, &format!("{material}_sheet"));
    push_candidate_path(out, &format!("sheet_{material}"));
}

fn push_storage_block_candidates(out: &mut Vec<FallbackItemCandidate>, material: &str) {
    push_candidate_id(out, &format!("minecraft:{material}_block"));
    push_candidate_path(out, &format!("{material}_block"));
    push_candidate_path(out, &format!("block_{material}"));
}

fn push_material_candidates(out: &mut Vec<FallbackItemCandidate>, material: &str, kind: &str) {
    push_candidate_id(out, &format!("minecraft:{material}_{kind}"));
    push_candidate_path(out, &format!("{material}_{kind}"));
    push_candidate_path(out, &format!("{kind}_{material}"));
}

fn push_candidate_id(out: &mut Vec<FallbackItemCandidate>, item_id: &str) {
    if !out
        .iter()
        .any(|candidate| matches!(candidate, FallbackItemCandidate::Id(id) if id == item_id))
    {
        out.push(FallbackItemCandidate::Id(item_id.to_string()));
    }
}

fn push_candidate_path(out: &mut Vec<FallbackItemCandidate>, item_path: &str) {
    if !out.iter().any(
        |candidate| matches!(candidate, FallbackItemCandidate::Path(path) if path == item_path),
    ) {
        out.push(FallbackItemCandidate::Path(item_path.to_string()));
    }
}

fn resolve_item_path_any_namespace(
    inst: &Instance,
    item_path: &str,
    display_item_id: &str,
) -> Result<Option<ItemIcon>> {
    for root in local_asset_roots(inst) {
        if let Some(icon) = resolve_item_path_from_dir(inst, &root, item_path, display_item_id)? {
            return Ok(Some(icon));
        }
    }
    for root in directory_asset_roots(&inst.resourcepacks_dir())? {
        if let Some(icon) = resolve_item_path_from_dir(inst, &root, item_path, display_item_id)? {
            return Ok(Some(icon));
        }
    }
    for pack in archive_asset_roots(&inst.resourcepacks_dir())? {
        if let Some(icon) = resolve_item_path_from_archive(inst, &pack, item_path, display_item_id)?
        {
            return Ok(Some(icon));
        }
    }
    for jar in version_asset_roots(inst) {
        if let Some(icon) = resolve_item_path_from_archive(inst, &jar, item_path, display_item_id)?
        {
            return Ok(Some(icon));
        }
    }
    for jar in archive_asset_roots(&inst.mods_dir())? {
        if let Some(icon) = resolve_item_path_from_archive(inst, &jar, item_path, display_item_id)?
        {
            return Ok(Some(icon));
        }
    }
    Ok(None)
}

fn resolve_item_path_from_dir(
    inst: &Instance,
    root: &Path,
    item_path: &str,
    display_item_id: &str,
) -> Result<Option<ItemIcon>> {
    for namespace in item_path_namespaces_from_dir(root, item_path)? {
        let item_id = format!("{namespace}:{item_path}");
        if let Some(icon) = resolve_item_icon_id(inst, &item_id, display_item_id)? {
            return Ok(Some(icon));
        }
    }
    Ok(None)
}

fn item_path_namespaces_from_dir(root: &Path, item_path: &str) -> Result<Vec<String>> {
    let assets = root.join("assets");
    let entries = match fs::read_dir(&assets) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(CoreError::io(assets, e)),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.with_path(&assets)?;
        let namespace_dir = entry.path();
        if !namespace_dir.is_dir() {
            continue;
        }
        let Some(namespace) = namespace_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if namespace_dir
            .join("models")
            .join("item")
            .join(format!("{item_path}.json"))
            .is_file()
            || namespace_dir
                .join("textures")
                .join("item")
                .join(format!("{item_path}.png"))
                .is_file()
        {
            out.push(namespace.to_string());
        }
    }
    out.sort();
    Ok(out)
}

fn resolve_item_path_from_archive(
    inst: &Instance,
    path: &Path,
    item_path: &str,
    display_item_id: &str,
) -> Result<Option<ItemIcon>> {
    for namespace in item_path_namespaces_from_archive(path, item_path)? {
        let item_id = format!("{namespace}:{item_path}");
        if let Some(icon) = resolve_item_icon_id(inst, &item_id, display_item_id)? {
            return Ok(Some(icon));
        }
    }
    Ok(None)
}

fn item_path_namespaces_from_archive(path: &Path, item_path: &str) -> Result<Vec<String>> {
    let file = fs::File::open(path).with_path(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| CoreError::Zip(e.to_string()))?;
    let mut out = Vec::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|e| CoreError::Zip(e.to_string()))?;
        let name = entry.name();
        if let Some(namespace) = item_path_namespace_from_asset_entry(name, item_path) {
            if !out.iter().any(|existing| existing == &namespace) {
                out.push(namespace);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn item_path_namespace_from_asset_entry(name: &str, item_path: &str) -> Option<String> {
    let rest = name.strip_prefix("assets/")?;
    let (namespace, rest) = rest.split_once('/')?;
    let model = format!("models/item/{item_path}.json");
    let texture = format!("textures/item/{item_path}.png");
    if rest == model || rest == texture {
        return Some(namespace.to_string());
    }
    None
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
    fn resolves_item_tag_icon_from_mod_jar_tag() {
        let temp = TempRoot::new("mod-jar-tag");
        let inst = Instance::new("pack", temp.path.clone());
        let paths = inst.paths();
        fs::create_dir_all(paths.version_dir("pack")).unwrap();
        fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
        fs::create_dir_all(inst.mods_dir()).unwrap();
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
        write_zip(
            &inst.mods_dir().join("forge-tags.jar"),
            &[(
                "data/forge/tags/items/nuggets/iron.json",
                br#"{"values":["minecraft:iron_nugget"]}"#,
            )],
        );

        let icon = resolve_item_icon(&inst, "#forge:nuggets/iron")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "#forge:nuggets/iron");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("1.19.2.jar"));
    }

    #[test]
    fn resolves_nested_item_tag_icon_from_mod_jar_tags() {
        let temp = TempRoot::new("mod-jar-nested-tag");
        let inst = Instance::new("pack", temp.path.clone());
        let paths = inst.paths();
        fs::create_dir_all(paths.version_dir("pack")).unwrap();
        fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
        fs::create_dir_all(inst.mods_dir()).unwrap();
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
                    "assets/minecraft/models/item/quartz.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/quartz"}}"#,
                ),
                ("assets/minecraft/textures/item/quartz.png", PNG),
            ],
        );
        write_zip(
            &inst.mods_dir().join("forge-tags.jar"),
            &[
                (
                    "data/forge/tags/items/gems/quartz.json",
                    br##"{"values":["#forge:gems/nether_quartz"]}"##,
                ),
                (
                    "data/forge/tags/items/gems/nether_quartz.json",
                    br#"{"values":[{"id":"minecraft:quartz","required":false}]}"#,
                ),
            ],
        );

        let icon = resolve_item_icon(&inst, "#forge:gems/quartz")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "#forge:gems/quartz");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("1.19.2.jar"));
    }

    #[test]
    fn prefers_stable_representative_for_broad_vanilla_item_tag() {
        let temp = TempRoot::new("vanilla-tag-representative");
        let inst = Instance::new("pack", temp.path.clone());
        let paths = inst.paths();
        fs::create_dir_all(paths.version_dir("pack")).unwrap();
        fs::create_dir_all(paths.version_dir("1.19.2")).unwrap();
        fs::create_dir_all(inst.mods_dir()).unwrap();
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
                    "data/minecraft/tags/items/planks.json",
                    br#"{"values":["minecraft:oak_planks"]}"#,
                ),
                (
                    "assets/minecraft/models/item/oak_planks.json",
                    br#"{"parent":"minecraft:block/oak_planks"}"#,
                ),
                (
                    "assets/minecraft/models/block/oak_planks.json",
                    br#"{"parent":"minecraft:block/cube_all","textures":{"all":"minecraft:block/oak_planks"}}"#,
                ),
                ("assets/minecraft/textures/block/oak_planks.png", PNG),
            ],
        );
        write_zip(
            &inst.mods_dir().join("custom-planks.jar"),
            &[
                (
                    "data/minecraft/tags/items/planks.json",
                    br#"{"values":["example:powdery_planks"]}"#,
                ),
                (
                    "assets/example/models/item/powdery_planks.json",
                    br#"{"parent":"example:block/powdery_planks"}"#,
                ),
                (
                    "assets/example/models/block/powdery_planks.json",
                    br#"{"parent":"minecraft:block/cube_all","textures":{"all":"example:block/powdery_planks"}}"#,
                ),
                ("assets/example/textures/block/powdery_planks.png", b"fake"),
            ],
        );

        let icon = resolve_item_icon(&inst, "#minecraft:planks")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "#minecraft:planks");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("1.19.2.jar"));
    }

    #[test]
    fn resolves_common_forge_item_tag_without_tag_json() {
        let temp = TempRoot::new("forge-tag-fallback");
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
                    "assets/minecraft/models/item/redstone.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"minecraft:item/redstone"}}"#,
                ),
                ("assets/minecraft/textures/item/redstone.png", PNG),
            ],
        );

        let icon = resolve_item_icon(&inst, "#forge:dusts/redstone")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "#forge:dusts/redstone");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("1.19.2.jar"));
    }

    #[test]
    fn resolves_common_forge_material_tag_across_mod_namespaces() {
        let temp = TempRoot::new("forge-tag-mod-namespace-fallback");
        let inst = Instance::new("pack", temp.path.clone());
        fs::create_dir_all(inst.mods_dir()).unwrap();
        write_zip(
            &inst.mods_dir().join("create.jar"),
            &[
                (
                    "assets/create/models/item/zinc_ingot.json",
                    br#"{"parent":"minecraft:item/generated","textures":{"layer0":"create:item/zinc_ingot"}}"#,
                ),
                ("assets/create/textures/item/zinc_ingot.png", PNG),
            ],
        );

        let icon = resolve_item_icon(&inst, "#forge:ingots/zinc")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "#forge:ingots/zinc");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("create.jar"));
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

    #[test]
    fn resolves_block_item_icon_from_particle_texture() {
        let temp = TempRoot::new("block-item-particle");
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
                    "assets/minecraft/models/item/crafting_table.json",
                    br#"{"parent":"minecraft:block/crafting_table"}"#,
                ),
                (
                    "assets/minecraft/models/block/crafting_table.json",
                    br#"{"parent":"minecraft:block/cube","textures":{"particle":"minecraft:block/crafting_table_front","north":"minecraft:block/crafting_table_front"}}"#,
                ),
                (
                    "assets/minecraft/textures/block/crafting_table_front.png",
                    PNG,
                ),
            ],
        );

        let icon = resolve_item_icon(&inst, "minecraft:crafting_table")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "minecraft:crafting_table");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("1.19.2.jar"));
    }

    #[test]
    fn resolves_mod_block_item_icon_from_particle_texture() {
        let temp = TempRoot::new("mod-block-item-particle");
        let inst = Instance::new("pack", temp.path.clone());
        fs::create_dir_all(inst.mods_dir()).unwrap();
        write_zip(
            &inst.mods_dir().join("create.jar"),
            &[
                (
                    "assets/create/models/item/mechanical_crafter.json",
                    br#"{"parent":"create:block/mechanical_crafter/item"}"#,
                ),
                (
                    "assets/create/models/block/mechanical_crafter/item.json",
                    br#"{"parent":"block/block","textures":{"particle":"create:block/brass_casing","4":"create:block/crafter_side"}}"#,
                ),
                ("assets/create/textures/block/brass_casing.png", PNG),
            ],
        );

        let icon = resolve_item_icon(&inst, "create:mechanical_crafter")
            .unwrap()
            .unwrap();

        assert_eq!(icon.item_id, "create:mechanical_crafter");
        assert_eq!(
            icon.data_url,
            format!("data:image/png;base64,{}", base64_encode(PNG))
        );
        assert!(icon.source.ends_with("create.jar"));
    }
}
