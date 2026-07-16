use super::*;

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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
    assert!(
        hit.document_id.starts_with("doc:"),
        "search hits should expose parent document ids for citations"
    );
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
    assert_eq!(opened.chunk.document_id, hit.document_id);
    assert!(opened.chunk.content.contains("Glowstone"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_multiple_roots_keep_duplicate_relative_paths_distinct() {
    let dir = temp_dir("wiki-multiple-roots");
    let first = dir.join("first");
    let second = dir.join("second");
    std::fs::create_dir_all(first.join("config")).unwrap();
    std::fs::create_dir_all(second.join("config")).unwrap();
    std::fs::write(first.join("config/guide.md"), "First guide has moonstone.").unwrap();
    std::fs::write(second.join("config/guide.md"), "Second guide has sunstone.").unwrap();

    let search = |query: &str| WikiSearchArgs {
        modpack_id: "multi-root-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![
            first.to_string_lossy().to_string(),
            second.to_string_lossy().to_string(),
        ],
        query: query.to_string(),
        top_k: Some(5),
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    };
    let moonstone = tool_wiki_search(search("moonstone")).await.unwrap();
    let sunstone = tool_wiki_search(search("sunstone")).await.unwrap();
    assert_eq!(moonstone.hits.len(), 1);
    assert_eq!(sunstone.hits.len(), 1);
    assert_ne!(moonstone.hits[0].chunk_id, sunstone.hits[0].chunk_id);
    assert_ne!(
        moonstone.hits[0].provenance.uri,
        sunstone.hits[0].provenance.uri
    );
    assert!(moonstone.hits[0].provenance.uri.starts_with("source://"));
    assert!(!serde_json::to_string(&moonstone)
        .unwrap()
        .contains(&dir.to_string_lossy().to_string()));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_archives_reject_absolute_and_parent_entry_names() {
    let dir = temp_dir("wiki-unsafe-archive-entry");
    std::fs::create_dir_all(&dir).unwrap();
    let archive = dir.join("unsafe.zip");
    std::fs::write(
        &archive,
        zip_bytes(&[
            (
                "C:/Users/Alice/private/notes.txt",
                b"windows-archive-secret",
            ),
            ("../private/notes.txt", b"parent-archive-secret"),
            (
                "docs/file:///Users/alice/private.txt",
                b"file-uri-archive-secret",
            ),
            (
                "docs/C:/Users/alice/private.txt",
                b"nested-drive-archive-secret",
            ),
            ("logs\\latest.txt", b"backslash-log-archive-secret"),
            ("mods\\private.txt", b"backslash-mod-archive-secret"),
            ("docs/safe.txt", b"safe archive moonstone guide"),
        ]),
    )
    .unwrap();

    for secret in [
        "windows-archive-secret",
        "parent-archive-secret",
        "file-uri-archive-secret",
        "nested-drive-archive-secret",
        "backslash-log-archive-secret",
        "backslash-mod-archive-secret",
    ] {
        let out = tool_wiki_search(WikiSearchArgs {
            modpack_id: "archive-pack".to_string(),
            instance_id: None,
            source_paths: vec![archive.to_string_lossy().to_string()],
            query: secret.to_string(),
            top_k: Some(5),
            kind: None,
            target_id: None,
            ingredient_id: None,
            include_structured: None,
        })
        .await
        .unwrap();
        assert!(
            out.hits.is_empty(),
            "unsafe archive entry was indexed: {secret}"
        );
    }

    let safe = tool_wiki_search(WikiSearchArgs {
        modpack_id: "archive-pack".to_string(),
        instance_id: None,
        source_paths: vec![archive.to_string_lossy().to_string()],
        query: "moonstone".to_string(),
        top_k: Some(5),
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    })
    .await
    .unwrap();
    assert_eq!(safe.hits.len(), 1);
    assert!(safe.hits[0].provenance.uri.starts_with("archive://"));
    assert!(!safe.hits[0].provenance.uri.contains("C:/Users"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_models_expose_only_relative_paths_with_provenance() {
    let dir = temp_dir("wiki-relative-paths");
    let config_dir = dir.join("config").join("guide");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("altar.md"),
        format!(
            r#"The moon altar consumes a starlight crystal. Internal root: {}
Other local paths: /Users/alice/.ssh/id_rsa C:\Users\alice\secret.txt \\nas\alice\pack ~/Library/private file:///Users/alice/private /opt/game/private.cfg
api_key=markdown-secret-sentinel
Minecraft commands: /give @p minecraft:diamond and /homecoming"#,
            dir.display()
        ),
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "path-safe-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "moon altar".to_string(),
        top_k: Some(5),
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    })
    .await
    .unwrap();

    let hit = out.hits.first().expect("local guide should be indexed");
    assert_eq!(hit.source_label, "config/guide/altar.md");
    assert_eq!(hit.provenance.uri, "config/guide/altar.md");
    assert_eq!(format!("{:?}", hit.provenance.trust), "Untrusted");
    let serialized = serde_json::to_string(&out).unwrap();
    assert!(!serialized.contains(&dir.to_string_lossy().to_string()));

    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "path-safe-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: hit.chunk_id.clone(),
    })
    .await
    .unwrap();
    assert_eq!(opened.chunk.provenance, hit.provenance);
    let serialized = serde_json::to_string(&opened).unwrap();
    assert!(!serialized.contains(&dir.to_string_lossy().to_string()));
    for private in [
        "/Users/alice",
        "C:\\\\Users\\\\alice",
        "\\\\\\\\nas\\\\alice",
        "~/Library",
        "file:///Users/alice",
        "/opt/game",
        "markdown-secret-sentinel",
    ] {
        assert!(
            !serialized.contains(private),
            "private value leaked: {private}"
        );
    }
    assert!(opened.chunk.content.contains("[LOCAL_PATH]"));
    assert!(opened.chunk.content.contains("api_key= [REDACTED]"));
    assert!(opened.chunk.content.contains("/give @p minecraft:diamond"));
    assert!(opened.chunk.content.contains("/homecoming"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_sources_redact_sensitive_keys_without_raw_parse_fallback() {
    let dir = temp_dir("wiki-secret-redaction");
    let config_dir = dir.join("config");
    let scripts_dir = dir.join("kubejs").join("server_scripts");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&scripts_dir).unwrap();
    std::fs::write(
        config_dir.join("service.json"),
        r#"{
            "display_name": "safe searchable service",
            "nested": { "access_token": "json-secret-sentinel" }
        }"#,
    )
    .unwrap();
    std::fs::write(
        config_dir.join("service.json5"),
        r#"{ safe: 1, access_token: "json5-secret-sentinel" }"#,
    )
    .unwrap();
    std::fs::write(
        config_dir.join("service.toml"),
        concat!(
            "name = \"safe toml service\"\n",
            "password = \"toml-secret-sentinel\"\n",
            "private_key = \"\"\"\n",
            "toml-multiline-secret-sentinel\n",
            "\"\"\"\n",
        ),
    )
    .unwrap();
    std::fs::write(
        scripts_dir.join("service.js"),
        concat!(
            "const apiToken = 'js-secret-sentinel'\n",
            "const discordToken = `\n",
            "js-multiline-secret-sentinel\n",
            "`\n",
            "const safeName = 'script service'\n",
        ),
    )
    .unwrap();
    std::fs::write(
        config_dir.join("service.yaml"),
        concat!(
            "name: safe yaml service\n",
            "password: |\n",
            "  yaml-multiline-secret-sentinel\n",
            "safe_setting: enabled\n",
        ),
    )
    .unwrap();
    std::fs::write(
        config_dir.join("service.properties"),
        "name=safe inline service; config: api_key=inline-secret-sentinel\n",
    )
    .unwrap();
    std::fs::write(
        config_dir.join("broken.json"),
        r#"{"password":"invalid-json-secret-sentinel""#,
    )
    .unwrap();

    for secret in [
        "json-secret-sentinel",
        "json5-secret-sentinel",
        "toml-secret-sentinel",
        "js-secret-sentinel",
        "toml-multiline-secret-sentinel",
        "js-multiline-secret-sentinel",
        "yaml-multiline-secret-sentinel",
        "inline-secret-sentinel",
        "invalid-json-secret-sentinel",
    ] {
        let out = tool_wiki_search(WikiSearchArgs {
            modpack_id: "secret-safe-pack".to_string(),
            instance_id: Some("local-instance".to_string()),
            source_paths: vec![dir.to_string_lossy().to_string()],
            query: secret.to_string(),
            top_k: Some(8),
            kind: None,
            target_id: None,
            ingredient_id: None,
            include_structured: None,
        })
        .await
        .unwrap();
        assert!(
            !serde_json::to_string(&out).unwrap().contains(secret),
            "secret leaked through wiki search output: {secret}"
        );
        for hit in out.hits {
            let opened = tool_wiki_open(WikiOpenArgs {
                modpack_id: "secret-safe-pack".to_string(),
                instance_id: Some("local-instance".to_string()),
                source_paths: vec![dir.to_string_lossy().to_string()],
                chunk_id: hit.chunk_id,
            })
            .await
            .unwrap();
            assert!(
                !serde_json::to_string(&opened).unwrap().contains(secret),
                "secret leaked through wiki open output: {secret}"
            );
        }
    }

    let safe = tool_wiki_search(WikiSearchArgs {
        modpack_id: "secret-safe-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "safe searchable service".to_string(),
        top_k: Some(5),
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    })
    .await
    .unwrap();
    let hit = safe
        .hits
        .first()
        .expect("non-sensitive JSON fields remain searchable");
    assert_eq!(format!("{:?}", hit.provenance.sensitivity), "Redacted");
    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "secret-safe-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: hit.chunk_id.clone(),
    })
    .await
    .unwrap();
    assert!(opened.chunk.content.contains("[REDACTED]"));

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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
async fn wiki_generic_collection_skips_internal_files_and_instance_metadata_is_allowlisted() {
    let dir = temp_dir("wiki-instance-allowlist");
    std::fs::create_dir_all(dir.join("mods")).unwrap();
    std::fs::write(dir.join("mods").join("safe-gameplay-mod.jar"), b"").unwrap();
    std::fs::write(
        dir.join("instance.json"),
        r#"{
            "name": "Allowlisted Pack",
            "memory_mb": 4096,
            "java_path": "/private/java-secret-sentinel/bin/java",
            "jvm_args": ["-Dcredential=jvm-secret-sentinel"],
            "game_args": ["--accessToken", "game-secret-sentinel"],
            "server": "server-secret-sentinel.example",
            "realm": {
                "realm_id": "realm-secret-sentinel",
                "code": "realm-code-secret-sentinel",
                "role": "member"
            },
            "tags": ["questing"]
        }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("wiki-corpus.json"),
        r#"{"cache_only":"cache-secret-sentinel"}"#,
    )
    .unwrap();

    let scope = WikiScope::from_modpack_entry("allowlist-pack".to_string(), None).unwrap();
    let corpus = WikiCorpus::from_sources(
        scope,
        vec![Box::new(LocalPathWikiSource::new(vec![dir.clone()]))],
    )
    .await
    .unwrap();
    for forbidden in [
        "java-secret-sentinel",
        "jvm-secret-sentinel",
        "game-secret-sentinel",
        "server-secret-sentinel",
        "realm-secret-sentinel",
        "realm-code-secret-sentinel",
        "cache-secret-sentinel",
    ] {
        let hits = corpus.search(forbidden, 8).await.unwrap();
        assert!(
            !serde_json::to_string(&hits).unwrap().contains(forbidden),
            "internal instance field leaked in search output: {forbidden}"
        );
        for hit in hits {
            let chunk = corpus.open(&hit.chunk_id).await.unwrap();
            assert!(
                !serde_json::to_string(&chunk).unwrap().contains(forbidden),
                "internal instance field leaked in open output: {forbidden}"
            );
        }
    }
    let safe = corpus.search("safe gameplay mod", 5).await.unwrap();
    let chunk = corpus.open(&safe[0].chunk_id).await.unwrap();
    assert!(chunk.content.contains("Instance name: Allowlisted Pack"));
    assert!(chunk.content.contains("Memory: 4096 MB"));
    assert!(!chunk.content.contains(&dir.to_string_lossy().to_string()));

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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
async fn wiki_search_builds_structured_ftb_quest_chunks_and_matches_typos() {
    let dir = temp_dir("wiki-structured-ftb-quest");
    let quests_dir = dir
        .join("config")
        .join("ftbquests")
        .join("quests")
        .join("chapters");
    std::fs::create_dir_all(&quests_dir).unwrap();
    std::fs::write(
        quests_dir.join("create_start.snbt"),
        r#"{
            title: "Create Start"
            quests: [{
                title: "Make a Crushing Wheel"
                subtitle: "Create automation"
                description: [
                    "Craft Andesite Alloy",
                    "Use Create stress units"
                ]
                tasks: [{
                    type: "item"
                    item: "create:crushing_wheel"
                }]
                rewards: [{ type: "item", item: "minecraft:diamond" }]
            }]
        }"#,
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "crushng whl".to_string(),
        top_k: Some(5),
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    })
    .await
    .unwrap();

    let hit = out
        .hits
        .first()
        .expect("typo query should still find the structured quest chunk");
    assert_eq!(hit.source_label, "generated:ftb-quests");
    assert!(hit.title.contains("Make a Crushing Wheel"));

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
        .contains("Quest title: Make a Crushing Wheel"));
    assert!(opened
        .chunk
        .content
        .contains("Quest token: create:crushing_wheel"));

    let _ = std::fs::remove_dir_all(&dir);
}
