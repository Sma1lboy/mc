//! "Start from scratch" base: synthesize an empty mrpack index for builds
//! that do not start from an existing base pack.

use std::io::{Cursor, Write};

use crate::error::{CoreError, Result};
use crate::loader::clean_loader_version;
use crate::modpack::formats::mrpack::{MrpackDependencies, MrpackIndex};
use mc_types::LoaderKind;

use super::*;

pub(super) fn scratch_base_index(approved: &ApprovedModpackBuild) -> MrpackIndex {
    let minecraft = optional_json_string(&approved.target, "minecraft_version");
    let loader = optional_json_string(&approved.target, "loader");
    let mut dependencies = MrpackDependencies {
        minecraft: minecraft.clone(),
        ..Default::default()
    };
    if let Some(loader) = loader.as_deref() {
        set_loader_dependency(
            &mut dependencies,
            loader,
            scratch_loader_dependency(loader, minecraft.as_deref(), &approved.target),
        );
    }
    MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: "agent-scratch".to_string(),
        name: optional_json_string(&approved.base_pack, "title")
            .unwrap_or_else(|| "Agent scratch modpack".to_string()),
        summary: Some("Generated from an empty base set by the local agent".to_string()),
        dependencies,
        files: Vec::new(),
    }
}

fn set_loader_dependency(
    dependencies: &mut MrpackDependencies,
    loader: &str,
    version: Option<String>,
) {
    match loader {
        "fabric" => dependencies.fabric_loader = version,
        "quilt" => dependencies.quilt_loader = version,
        "forge" => dependencies.forge = version,
        "neoforge" => dependencies.neoforge = version,
        _ => {}
    }
}

fn scratch_loader_dependency(
    loader: &str,
    mc_version: Option<&str>,
    target: &serde_json::Value,
) -> Option<String> {
    target_loader_dependency(loader, mc_version, target)
        .or_else(|| known_scratch_loader_dependency(loader, mc_version).map(str::to_string))
}

fn target_loader_dependency(
    loader: &str,
    mc_version: Option<&str>,
    target: &serde_json::Value,
) -> Option<String> {
    let loader = loader.trim().to_ascii_lowercase();
    let mc_version = mc_version.unwrap_or_default();
    let kind = loader_kind(&loader)?;
    ["loader_version", "version_id"]
        .into_iter()
        .filter_map(|field| optional_json_string(target, field))
        .map(|version| clean_loader_version(&version, kind, mc_version))
        .find(|version| concrete_loader_dependency(version))
}

fn loader_kind(loader: &str) -> Option<LoaderKind> {
    match loader {
        "fabric" => Some(LoaderKind::Fabric),
        "quilt" => Some(LoaderKind::Quilt),
        "forge" => Some(LoaderKind::Forge),
        "neoforge" => Some(LoaderKind::NeoForge),
        _ => None,
    }
}

fn concrete_loader_dependency(version: &str) -> bool {
    let trimmed = version.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("latest")
        && trimmed.chars().any(|c| c.is_ascii_digit())
}

fn known_scratch_loader_dependency(loader: &str, mc_version: Option<&str>) -> Option<&'static str> {
    match (loader.trim().to_ascii_lowercase().as_str(), mc_version?) {
        ("fabric", _) => Some("0.16.14"),
        ("quilt", _) => Some("0.26.4"),
        ("forge", "1.20.1") => Some("47.2.0"),
        ("forge", "1.20.2") => Some("48.1.0"),
        ("forge", "1.20.4") => Some("49.0.31"),
        ("neoforge", "1.20.1") => Some("47.1.106"),
        ("neoforge", "1.20.4") => Some("20.4.237"),
        _ => None,
    }
}

pub(super) fn scratch_base_archive_bytes(index: &MrpackIndex) -> Result<Vec<u8>> {
    let index_bytes = serde_json::to_vec_pretty(index).map_err(|source| CoreError::Parse {
        what: "scratch modrinth.index.json".to_string(),
        source,
    })?;
    let mut output = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file("modrinth.index.json", options)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    writer
        .write_all(&index_bytes)
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    writer.finish().map_err(|e| CoreError::Zip(e.to_string()))?;
    Ok(output.into_inner())
}

