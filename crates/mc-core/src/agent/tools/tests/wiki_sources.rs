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
