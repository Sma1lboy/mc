//! Unit tests for the deterministic modpack tools.
//!
//! Each `tool_*` runs against an in-memory `FakeChatProvider` — no live API key,
//! no network (the archive/build tests spin up throwaway localhost servers).

use std::collections::HashMap;

use crate::modplatform::Dependency;

use super::fake_provider::{
    bytes_server, cdn_file, ctx_of, hit, registry_of, temp_dir, version, zip_index,
    FakeChatProvider,
};
use super::{
    tool_build_modpack, tool_inspect_base_modpack, tool_mod_get_detail, tool_resolve_mods,
    tool_search_base_modpacks, tool_search_mods, BuildModRef, BuildModpackArgs, BuildTarget,
    ChatToolsCtx, InspectBaseModpackArgs, ModGetDetailArgs, ResolveModsArgs,
    SearchBaseModpacksArgs, SearchModsArgs,
};

// ---------------------------------------------------------------------------
// Tool tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_base_modpacks_maps_provider_hits() {
    let provider = FakeChatProvider {
        search_hits: vec![hit("packid", "cool-pack", "Cool Pack")],
        ..Default::default()
    };
    let out = tool_search_base_modpacks(
        &ctx_of(provider),
        SearchBaseModpacksArgs {
            query: "tech exploration".to_string(),
            mc_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
        })
        .await
        .unwrap();
    assert_eq!(out.candidates.len(), 1);
    let c = &out.candidates[0];
    assert_eq!(c.provider, "modrinth");
    assert_eq!(c.project_id, "packid");
    assert_eq!(c.slug, "cool-pack");
    assert_eq!(c.title, "Cool Pack");
    assert_eq!(c.downloads, 4242);
}

#[tokio::test]
async fn search_mods_maps_provider_hits() {
    let provider = FakeChatProvider {
        search_hits: vec![hit("sodium", "sodium", "Sodium")],
        ..Default::default()
    };
    let out = tool_search_mods(
        &ctx_of(provider),
        SearchModsArgs {
            query: "performance".to_string(),
            mc_version: "1.20.1".to_string(),
            loader: "fabric".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(out.mods.len(), 1);
    assert_eq!(out.mods[0].project_id, "sodium");
    assert_eq!(out.mods[0].provider, "modrinth");
}

#[tokio::test]
async fn resolve_mods_walks_required_dependencies() {
    let mut versions = HashMap::new();
    versions.insert(
        "root".to_string(),
        vec![version(
            "root-v1",
            cdn_file("root"),
            vec![Dependency {
                project_id: Some("dep".to_string()),
                version_id: None,
                dependency_type: "required".to_string(),
            }],
        )],
    );
    versions.insert(
        "dep".to_string(),
        vec![version("dep-v1", cdn_file("dep"), Vec::new())],
    );
    let provider = FakeChatProvider {
        versions,
        ..Default::default()
    };
    let out = tool_resolve_mods(
        &ctx_of(provider),
        ResolveModsArgs {
            project_ids: vec!["root".to_string()],
            mc_version: "1.20.1".to_string(),
            loader: "fabric".to_string(),
            already_installed: None,
        })
        .await
        .unwrap();

    let mut ids: Vec<_> = out.resolved.iter().map(|r| r.project_id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["dep".to_string(), "root".to_string()]);
    assert!(out.unresolved.is_empty());
    // Resolved refs carry real version ids + urls echoed straight from the provider.
    let root = out.resolved.iter().find(|r| r.project_id == "root").unwrap();
    assert_eq!(root.version_id, "root-v1");
    assert!(root.url.starts_with("https://cdn.modrinth.com/data/root/"));
}

#[tokio::test]
async fn resolve_mods_honors_already_installed() {
    let mut versions = HashMap::new();
    versions.insert(
        "root".to_string(),
        vec![version("root-v1", cdn_file("root"), Vec::new())],
    );
    let provider = FakeChatProvider {
        versions,
        ..Default::default()
    };
    let out = tool_resolve_mods(
        &ctx_of(provider),
        ResolveModsArgs {
            project_ids: vec!["root".to_string()],
            mc_version: "1.20.1".to_string(),
            loader: "fabric".to_string(),
            already_installed: Some(vec!["modrinth:root".to_string()]),
        })
        .await
        .unwrap();
    assert!(out.resolved.is_empty(), "already-installed root should not be resolved again");
}

#[tokio::test]
async fn mod_get_detail_returns_project_and_capped_versions() {
    let mut versions = HashMap::new();
    // 12 published versions -> only the 10 newest (provider order) survive the cap.
    versions.insert(
        "sodium".to_string(),
        (0..12)
            .map(|i| {
                version(
                    &format!("sodium-v{i}"),
                    cdn_file("sodium"),
                    if i == 0 {
                        vec![Dependency {
                            project_id: Some("dep".to_string()),
                            version_id: None,
                            dependency_type: "required".to_string(),
                        }]
                    } else {
                        Vec::new()
                    },
                )
            })
            .collect(),
    );
    let mut projects = HashMap::new();
    let mut sodium_hit = hit("sodium", "sodium", "Sodium");
    sodium_hit.categories = vec!["optimization".to_string()];
    projects.insert("sodium".to_string(), sodium_hit);

    let provider = FakeChatProvider {
        versions,
        projects,
        ..Default::default()
    };
    let out = tool_mod_get_detail(
        &ctx_of(provider),
        ModGetDetailArgs {
            provider: None,
            project_id: "sodium".to_string(),
            minecraft_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(out.project.title, "Sodium");
    assert_eq!(out.project.slug, "sodium");
    assert_eq!(out.project.downloads, 4242);
    assert_eq!(out.project.categories, vec!["optimization".to_string()]);

    assert_eq!(out.versions.len(), 10, "version list must be capped");
    let first = &out.versions[0];
    assert_eq!(first.version_id, "sodium-v0");
    assert_eq!(first.version_number, "1.0.0");
    assert_eq!(first.game_versions, vec!["1.20.1".to_string()]);
    assert_eq!(first.loaders, vec!["fabric".to_string()]);
    assert_eq!(first.dependencies_count, 1);
    assert_eq!(first.filename.as_deref(), Some("sodium.jar"));
}

#[tokio::test]
async fn inspect_base_modpack_parses_modlist_and_enriches() {
    // Minimal .mrpack referencing one Modrinth project via its CDN download url.
    let index = serde_json::json!({
        "formatVersion": 1,
        "game": "minecraft",
        "versionId": "1.0.0",
        "name": "Base Pack",
        "dependencies": { "minecraft": "1.20.1", "fabric-loader": "0.15.7" },
        "files": [{
            "path": "mods/sodium.jar",
            "downloads": ["https://cdn.modrinth.com/data/sodium/versions/v/sodium.jar"],
            "hashes": { "sha512": "h" },
            "fileSize": 100
        }]
    });
    let archive = zip_index(serde_json::to_vec(&index).unwrap());
    let archive_url = bytes_server(archive);

    let mut base_file = cdn_file("basepack");
    base_file.url = archive_url;
    let mut versions = HashMap::new();
    versions.insert("basepack".to_string(), vec![version("basepack-v1", base_file, Vec::new())]);

    let mut projects = HashMap::new();
    let mut sodium_hit = hit("sodium", "sodium", "Sodium");
    sodium_hit.categories = vec!["optimization".to_string()];
    projects.insert("sodium".to_string(), sodium_hit);

    let provider = FakeChatProvider {
        versions,
        projects,
        ..Default::default()
    };
    let out = tool_inspect_base_modpack(
        &ctx_of(provider),
        InspectBaseModpackArgs {
            project_id: "basepack".to_string(),
            mc_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
        })
        .await
        .unwrap();

    assert_eq!(out.mod_count, 1);
    assert_eq!(out.mods.len(), 1);
    assert_eq!(out.mods[0].title, "Sodium");
    assert_eq!(out.covered_features, vec!["optimization".to_string()]);
    assert_eq!(out.mc_version.as_deref(), Some("1.20.1"));
}

#[tokio::test]
async fn build_modpack_from_scratch_writes_verified_mrpack() {
    let mut versions = HashMap::new();
    versions.insert(
        "sodium".to_string(),
        vec![version("sodium-v1", cdn_file("sodium"), Vec::new())],
    );
    let provider = FakeChatProvider {
        versions,
        ..Default::default()
    };
    let out_dir = temp_dir("build");
    let ctx = ChatToolsCtx::new(registry_of(provider), out_dir.clone());
    let out = tool_build_modpack(
        &ctx,
        BuildModpackArgs {
            target: BuildTarget {
                mc_version: "1.20.1".to_string(),
                loader: "fabric".to_string(),
            },
            base_pack: None,
            extra_mods: vec![BuildModRef {
                provider: Some("modrinth".to_string()),
                project_id: "sodium".to_string(),
                version_id: "sodium-v1".to_string(),
                title: Some("Sodium".to_string()),
            }],
            // A path-traversal attempt: it must be reduced to a bare basename
            // inside the sandbox, never escaping output_dir.
            output_filename: "../../my pack".to_string(),
        })
        .await
        .unwrap();

    // The executor writes, then re-verifies the archive before reporting done.
    assert_eq!(out.status, "completed", "manifest: {}", out.manifest);
    let raw = out.output_path.expect("output path");
    let path = std::path::Path::new(&raw);
    assert_eq!(
        path.parent(),
        Some(out_dir.as_path()),
        "build must stay inside the sandbox output dir: {raw}"
    );
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        "my pack.mrpack",
        "traversal segments must be stripped to a bare basename: {raw}"
    );
    assert!(path.exists(), "mrpack should be on disk");
    let _ = std::fs::remove_dir_all(&out_dir);
}

