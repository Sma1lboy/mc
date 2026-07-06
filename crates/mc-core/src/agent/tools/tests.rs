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
    prebuild_wiki_corpus_cache, tool_build_modpack, tool_inspect_base_modpack, tool_mod_get_detail,
    tool_resolve_mods, tool_search_base_modpacks, tool_search_mods, tool_wiki_open,
    tool_wiki_search, wiki_corpus_cache_path, BuildModRef, BuildModpackArgs, BuildTarget,
    ChatToolsCtx, InspectBaseModpackArgs, LocalPathWikiSource, ModGetDetailArgs, ResolveModsArgs,
    SearchBaseModpacksArgs, SearchModsArgs, WikiCorpus, WikiOpenArgs, WikiScope, WikiSearchArgs,
};

// ---------------------------------------------------------------------------
// Tool tests
// ---------------------------------------------------------------------------

fn zip_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::{Cursor, Write};

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn rewrite_first_cached_wiki_chunk(dir: &std::path::Path, content: &str) {
    let cache_path = wiki_corpus_cache_path(dir);
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache_path).unwrap()).unwrap();
    value["chunks"][0]["title"] = serde_json::Value::String("Cached wiki chunk".to_string());
    value["chunks"][0]["content"] = serde_json::Value::String(content.to_string());
    std::fs::write(&cache_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

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
        },
    )
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
        },
    )
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
        },
    )
    .await
    .unwrap();

    let mut ids: Vec<_> = out.resolved.iter().map(|r| r.project_id.clone()).collect();
    ids.sort();
    assert_eq!(ids, vec!["dep".to_string(), "root".to_string()]);
    assert!(out.unresolved.is_empty());
    // Resolved refs carry real version ids + urls echoed straight from the provider.
    let root = out
        .resolved
        .iter()
        .find(|r| r.project_id == "root")
        .unwrap();
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
        },
    )
    .await
    .unwrap();
    assert!(
        out.resolved.is_empty(),
        "already-installed root should not be resolved again"
    );
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
        },
    )
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
    versions.insert(
        "basepack".to_string(),
        vec![version("basepack-v1", base_file, Vec::new())],
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
    let out = tool_inspect_base_modpack(
        &ctx_of(provider),
        InspectBaseModpackArgs {
            project_id: "basepack".to_string(),
            mc_version: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
        },
    )
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
        },
    )
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

#[tokio::test]
async fn install_modpack_rejects_paths_outside_sandbox() {
    use crate::download::Downloader;
    use crate::modpack::import::ImportEngine;
    use crate::modplatform::provider::ProviderRegistry;

    use super::{tool_install_modpack, InstallModpackArgs};

    let sandbox = temp_dir("install-sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let outside = temp_dir("install-outside");
    std::fs::create_dir_all(&outside).unwrap();
    let evil = outside.join("evil.mrpack");
    std::fs::write(&evil, b"not a real pack").unwrap();

    let ctx = ChatToolsCtx::new(registry_of(FakeChatProvider::default()), sandbox.clone());
    let engine = ImportEngine::with_defaults(Downloader::new(2).unwrap(), ProviderRegistry::new());
    let err = tool_install_modpack(
        &ctx,
        &engine,
        &outside,
        InstallModpackArgs {
            path: evil.to_string_lossy().to_string(),
        },
    )
    .await
    .expect_err("a path outside the agent output dir must be rejected");
    assert!(err.0.contains("outside"), "unexpected error: {}", err.0);

    let _ = std::fs::remove_dir_all(&sandbox);
    let _ = std::fs::remove_dir_all(&outside);
}

#[tokio::test]
async fn wiki_search_reads_local_text_sources_and_opens_chunks() {
    let dir = temp_dir("wiki-text");
    let wiki_dir = dir.join("config").join("ftbquests").join("quests");
    std::fs::create_dir_all(&wiki_dir).unwrap();
    let source = wiki_dir.join("the_aether.snbt");
    std::fs::write(
        &source,
        "The Aether portal is built with Glowstone.\nIt is lit with a water bucket.\n",
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "better-mc".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "aether portal".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();

    assert_eq!(
        out.scope.corpus_id,
        "modpack:better-mc:instance:local-instance"
    );
    assert_eq!(out.source_count, 1);
    let hit = out
        .hits
        .iter()
        .find(|hit| hit.source_label.ends_with("the_aether.snbt"))
        .expect("raw quest source should be searchable");
    assert!(hit.snippet.contains("Aether portal"));

    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "better-mc".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: hit.chunk_id.clone(),
    })
    .await
    .unwrap();

    assert_eq!(opened.chunk.chunk_id, hit.chunk_id);
    assert!(opened.chunk.content.contains("Glowstone"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_includes_generated_instance_data() {
    let dir = temp_dir("wiki-instance-data");
    std::fs::create_dir_all(dir.join("mods")).unwrap();
    std::fs::write(dir.join("mods").join("sodium-fabric.jar"), b"").unwrap();
    std::fs::write(
        dir.join("instance.json"),
        r#"{
            "name": "Better MC",
            "source": {
                "provider": "modrinth",
                "project_id": "better-mc",
                "version_id": "v1"
            },
            "tags": ["questing"]
        }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("mmc-pack.json"),
        r#"{
            "formatVersion": 1,
            "components": [
                { "uid": "net.minecraft", "version": "1.20.1", "important": true },
                { "uid": "net.fabricmc.fabric-loader", "version": "0.15.7", "important": true }
            ]
        }"#,
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "better-mc".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "sodium".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();

    assert_eq!(out.source_count, 1);
    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].source_label, "generated:instance-data");

    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "better-mc".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: out.hits[0].chunk_id.clone(),
    })
    .await
    .unwrap();

    assert!(opened.chunk.content.contains("Instance name: Better MC"));
    assert!(opened
        .chunk
        .content
        .contains("net.fabricmc.fabric-loader: 0.15.7"));
    assert!(opened.chunk.content.contains("sodium-fabric.jar"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_reads_complete_ftb_quest_sources() {
    let dir = temp_dir("wiki-ftb-quests");
    let quests_dir = dir
        .join("config")
        .join("ftbquests")
        .join("quests")
        .join("chapters");
    std::fs::create_dir_all(&quests_dir).unwrap();
    std::fs::write(
        quests_dir.join("getting_started.snbt"),
        r#"{
            title: "Getting Started"
            quests: [{
                title: "Make a Crushing Wheel"
                subtitle: "Create automation"
                description: ["Craft Andesite Alloy", "Use Create stress units"]
                tasks: [{ type: "item", item: "create:crushing_wheel" }]
                rewards: [{ type: "item", item: "minecraft:diamond" }]
                dependencies: ["long_unique_gate"]
            }]
        }"#,
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "crushing wheel".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();

    let hit = out
        .hits
        .iter()
        .find(|hit| hit.source_label == "generated:ftb-quests")
        .expect("FTB quest source should be searchable through generated source");
    assert!(hit
        .title
        .contains("config/ftbquests/quests/chapters/getting_started.snbt"));

    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: hit.chunk_id.clone(),
    })
    .await
    .unwrap();

    assert!(opened
        .chunk
        .content
        .contains("FTB Quests source file: config/ftbquests/quests/chapters/getting_started.snbt"));
    assert!(opened.chunk.content.contains(r#"title: "Getting Started""#));
    assert!(opened
        .chunk
        .content
        .contains(r#"title: "Make a Crushing Wheel""#));
    assert!(opened
        .chunk
        .content
        .contains(r#"description: ["Craft Andesite Alloy", "Use Create stress units"]"#));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_and_open_use_valid_corpus_cache() {
    let dir = temp_dir("wiki-cache-hit");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("guide.md"),
        "The real source mentions only starlight.",
    )
    .unwrap();

    prebuild_wiki_corpus_cache(
        "better-mc".to_string(),
        Some("local-instance".to_string()),
        &dir,
    )
    .await
    .unwrap();
    rewrite_first_cached_wiki_chunk(&dir, "cached sentinel answer from persisted corpus");

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "better-mc".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "sentinel".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();

    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].title, "Cached wiki chunk");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_fingerprint_ignores_runtime_dirs_and_uses_cache() {
    let dir = temp_dir("wiki-cache-runtime-dirs");
    std::fs::create_dir_all(dir.join("logs")).unwrap();
    std::fs::create_dir_all(dir.join("saves").join("world").join("region")).unwrap();
    std::fs::write(
        dir.join("guide.md"),
        "The stable source mentions moonstone.",
    )
    .unwrap();
    std::fs::write(dir.join("logs").join("latest.log"), "first launch").unwrap();
    std::fs::write(
        dir.join("saves")
            .join("world")
            .join("region")
            .join("r.0.0.mca"),
        "binary-ish save data",
    )
    .unwrap();

    prebuild_wiki_corpus_cache(
        "runtime-pack".to_string(),
        Some("local-instance".to_string()),
        &dir,
    )
    .await
    .unwrap();
    rewrite_first_cached_wiki_chunk(&dir, "cached runtime sentinel");
    std::fs::write(
        dir.join("logs").join("latest.log"),
        "rotated log with volatile terms",
    )
    .unwrap();
    std::fs::write(
        dir.join("saves")
            .join("world")
            .join("region")
            .join("r.0.1.mca"),
        "new save data",
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "runtime-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "sentinel".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();
    assert_eq!(
        out.hits.len(),
        1,
        "runtime-only changes must not invalidate cache"
    );

    let volatile = tool_wiki_search(WikiSearchArgs {
        modpack_id: "runtime-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "volatile".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();
    assert!(volatile.hits.is_empty(), "logs/saves must not be indexed");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_open_chunk_id_survives_corpus_reordering() {
    let dir = temp_dir("wiki-stable-chunk-id");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("b.md"), "Second guide has the citrine altar.").unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "stable-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "citrine".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();
    let chunk_id = out.hits[0].chunk_id.clone();

    std::fs::write(dir.join("a.md"), "First guide has unrelated copper ore.").unwrap();
    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "stable-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id,
    })
    .await
    .unwrap();
    assert!(opened.chunk.content.contains("citrine altar"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[tokio::test]
async fn wiki_search_skips_symlinks() {
    use std::os::unix::fs::symlink;

    let dir = temp_dir("wiki-symlink-root");
    let outside = temp_dir("wiki-symlink-outside");
    std::fs::create_dir_all(dir.join("config")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(dir.join("guide.md"), "Local guide mentions obsidian").unwrap();
    std::fs::write(outside.join("secret.md"), "outside secret sentinel").unwrap();
    symlink(&outside, dir.join("config").join("external")).unwrap();
    symlink(outside.join("secret.md"), dir.join("linked-secret.md")).unwrap();
    symlink(&dir, dir.join("config").join("loop")).unwrap();

    let secret = tool_wiki_search(WikiSearchArgs {
        modpack_id: "safe-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "secret sentinel".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();
    assert!(
        secret.hits.is_empty(),
        "symlink targets outside the instance must not be indexed"
    );

    let local = tool_wiki_search(WikiSearchArgs {
        modpack_id: "safe-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "obsidian".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();
    assert_eq!(local.hits.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}

#[tokio::test]
async fn wiki_search_applies_archive_total_budget() {
    let dir = temp_dir("wiki-archive-budget");
    std::fs::create_dir_all(&dir).unwrap();
    let archive = dir.join("wiki-source.mrpack");
    let early = vec![b'a'; 32 * 1024];
    let late = b"The late archive entry mentions forbidden sentinel content.";
    let mut entries: Vec<(String, Vec<u8>)> = (0..40)
        .map(|i| (format!("overrides/config/wiki/{i:02}.txt"), early.clone()))
        .collect();
    entries.push((
        "overrides/config/wiki/zz-late.txt".to_string(),
        late.to_vec(),
    ));
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    std::fs::write(&archive, zip_bytes(&refs)).unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "budget-pack".to_string(),
        instance_id: None,
        source_paths: vec![archive.to_string_lossy().to_string()],
        query: "forbidden sentinel".to_string(),
        top_k: Some(5),
    })
    .await
    .unwrap();
    assert!(
        out.hits.is_empty(),
        "archive reader must stop before unbounded total extraction"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_corpus_searches_through_unified_source_interface() {
    let dir = temp_dir("wiki-source-interface");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("guide.md"),
        "The star altar consumes starlight and aquamarine.",
    )
    .unwrap();
    let scope = WikiScope::from_modpack_entry("astral-pack".to_string(), None).unwrap();
    let local = LocalPathWikiSource::new(vec![dir.clone()]);

    let corpus = WikiCorpus::from_sources(scope.clone(), vec![Box::new(local)])
        .await
        .unwrap();
    let hits = corpus.search("star altar", 5).await.unwrap();
    let opened = corpus.open(&hits[0].chunk_id).await.unwrap();

    assert_eq!(corpus.scope(), &scope);
    assert_eq!(corpus.source_count(), 1);
    assert!(hits[0].snippet.contains("star altar"));
    assert!(opened.content.contains("aquamarine"));

    let _ = std::fs::remove_dir_all(&dir);
}
