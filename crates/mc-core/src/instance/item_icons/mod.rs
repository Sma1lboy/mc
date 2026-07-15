//! Resolve Minecraft item ids to local icon images for installed instances.
//!
//! This is deliberately best-effort: recipe cards can still render item labels
//! when a texture cannot be resolved.

mod assets;
mod tags;

#[cfg(test)]
mod tests;

use serde::Serialize;

use super::Instance;
use crate::error::Result;

use assets::{
    archive_asset_roots, directory_asset_roots, local_asset_roots, parse_item_id,
    resolve_from_archive, resolve_from_dir, version_asset_roots, ItemQuery,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct ItemIcon {
    pub item_id: String,
    pub data_url: String,
    pub source: String,
}

pub fn resolve_item_icon(inst: &Instance, item_id: &str) -> Result<Option<ItemIcon>> {
    let item_id = item_id.trim();
    if item_id.starts_with('#') {
        return tags::resolve_tag_icon(inst, item_id);
    }
    resolve_item_icon_id(inst, item_id, item_id)
}

pub(super) fn resolve_item_icon_id(
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
