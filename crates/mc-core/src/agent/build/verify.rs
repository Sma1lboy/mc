//! Post-write verification: re-open the written `.mrpack` and check the index,
//! target, and overrides against the approved plan.

use std::collections::HashSet;
use std::path::Path;

use crate::error::{CoreError, Result};
use crate::modpack::formats::mrpack::MrpackIndex;

use super::*;

pub(super) fn verify_written_mrpack(
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

fn verify_written_mrpack_index(index: &MrpackIndex, approved: &ApprovedModpackBuild) -> Result<()> {
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

fn verify_written_mrpack_target(
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

fn verify_written_mrpack_overrides(
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

fn mrpack_index_path_is_safe(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    !normalized.is_empty()
        && !normalized.starts_with('/')
        && normalized
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

