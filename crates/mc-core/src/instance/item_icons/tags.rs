use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::assets::{
    archive_asset_roots, directory_asset_roots, local_asset_roots, parse_item_id,
    read_archive_text, version_asset_roots,
};
use super::{resolve_item_icon_id, ItemIcon};
use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;

struct TagQuery {
    namespace: String,
    path: String,
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
fn local_data_roots(inst: &Instance) -> Vec<PathBuf> {
    vec![
        inst.game_dir().join("kubejs"),
        inst.datapacks_dir(),
        inst.game_dir(),
    ]
}

pub(super) fn resolve_tag_icon(inst: &Instance, tag_id: &str) -> Result<Option<ItemIcon>> {
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

    if let Some(icon) = resolve_vanilla_tag_icon(inst, tag_id, display_item_id, visited, depth)? {
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

fn resolve_vanilla_tag_icon(
    inst: &Instance,
    tag_id: &str,
    display_item_id: &str,
    visited: &mut HashSet<String>,
    depth: usize,
) -> Result<Option<ItemIcon>> {
    let Some((namespace, _)) = parse_tag_id(tag_id) else {
        return Ok(None);
    };
    if namespace != "minecraft" {
        return Ok(None);
    }
    for value in read_vanilla_item_tag_values(inst, tag_id)? {
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
    Ok(None)
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

fn read_vanilla_item_tag_values(inst: &Instance, tag_id: &str) -> Result<Vec<TagValue>> {
    let Some((namespace, path)) = parse_tag_id(tag_id) else {
        return Ok(Vec::new());
    };
    if namespace != "minecraft" {
        return Ok(Vec::new());
    }
    let query = TagQuery { namespace, path };
    let mut out = Vec::new();

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
