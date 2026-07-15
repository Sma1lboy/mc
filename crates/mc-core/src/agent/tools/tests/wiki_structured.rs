use super::*;

#[tokio::test]
async fn wiki_search_extracts_structured_recipe_documents_from_mod_jars() {
    let dir = temp_dir("wiki-mod-jar-recipes");
    let mods_dir = dir.join("mods");
    std::fs::create_dir_all(&mods_dir).unwrap();
    std::fs::write(
        mods_dir.join("create.jar"),
        zip_bytes(&[
            (
                "assets/create/lang/en_us.json",
                br#"{
                    "block.create.andesite_casing": "Andesite Casing",
                    "item.create.andesite_alloy": "Andesite Alloy"
                }"#,
            ),
            (
                "data/create/recipes/crafting/andesite_casing.json",
                br#"{
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["PPP", "PAP", "PPP"],
                    "key": {
                        "P": { "item": "minecraft:oak_planks" },
                        "A": { "item": "create:andesite_alloy" }
                    },
                    "result": { "item": "create:andesite_casing", "count": 1 }
                }"#,
            ),
        ]),
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "andesite casing recipe".to_string(),
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
        .find(|hit| hit.kind.as_deref() == Some("recipe"))
        .expect("recipe JSON inside mod jar should become a structured wiki hit");
    assert!(hit.title.contains("Andesite Casing"));
    assert_eq!(hit.source_label, "generated:recipe");
    assert_eq!(
        hit.structured
            .as_ref()
            .and_then(|value| value.pointer("/result/id"))
            .and_then(|value| value.as_str()),
        Some("create:andesite_casing")
    );

    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: hit.chunk_id.clone(),
    })
    .await
    .unwrap();
    assert_eq!(opened.chunk.kind.as_deref(), Some("recipe"));
    assert!(opened.chunk.content.contains("kind: recipe"));
    assert!(opened.chunk.content.contains("create:andesite_alloy"));
    assert_eq!(
        opened
            .chunk
            .structured
            .as_ref()
            .and_then(|value| value.pointer("/result/label"))
            .and_then(|value| value.as_str()),
        Some("Andesite Casing")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_filters_structured_recipes_by_target_and_ingredient() {
    let dir = temp_dir("wiki-recipe-structured-filters");
    let mods_dir = dir.join("mods");
    std::fs::create_dir_all(&mods_dir).unwrap();
    std::fs::write(
        mods_dir.join("create.jar"),
        zip_bytes(&[
            (
                "assets/create/lang/en_us.json",
                br#"{
                    "item.create.andesite_alloy": "Andesite Alloy",
                    "block.create.andesite_casing": "Andesite Casing"
                }"#,
            ),
            (
                "data/create/recipes/materials/andesite_alloy.json",
                br#"{
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["AB", "BA"],
                    "key": {
                        "A": { "item": "minecraft:andesite" },
                        "B": { "tag": "forge:nuggets/iron" }
                    },
                    "result": { "item": "create:andesite_alloy" }
                }"#,
            ),
            (
                "data/create/recipes/crafting/andesite_casing.json",
                br#"{
                    "type": "minecraft:crafting_shaped",
                    "pattern": ["PPP", "PAP", "PPP"],
                    "key": {
                        "P": { "tag": "minecraft:planks" },
                        "A": { "item": "create:andesite_alloy" }
                    },
                    "result": { "item": "create:andesite_casing" }
                }"#,
            ),
        ]),
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "recipe".to_string(),
        top_k: Some(5),
        kind: Some("recipe".to_string()),
        target_id: Some("create:andesite_casing".to_string()),
        ingredient_id: Some("create:andesite_alloy".to_string()),
        include_structured: Some(false),
    })
    .await
    .unwrap();

    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].kind.as_deref(), Some("recipe"));
    assert!(out.hits[0].title.contains("Andesite Casing"));
    assert!(
        out.hits[0].structured.is_none(),
        "include_structured=false should strip structured payloads from hits"
    );

    let exact_out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: " ".to_string(),
        top_k: Some(5),
        kind: Some("recipe".to_string()),
        target_id: Some("create:andesite_casing".to_string()),
        ingredient_id: None,
        include_structured: Some(true),
    })
    .await
    .unwrap();

    assert_eq!(exact_out.hits.len(), 1);
    assert_eq!(
        exact_out.hits[0]
            .structured
            .as_ref()
            .and_then(|value| value.pointer("/result/id"))
            .and_then(|value| value.as_str()),
        Some("create:andesite_casing")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_indexes_kubejs_recipe_overrides_ahead_of_mod_recipes() {
    let dir = temp_dir("wiki-kubejs-recipe-overrides");
    let mods_dir = dir.join("mods");
    let scripts_dir = dir.join("kubejs").join("server_scripts");
    std::fs::create_dir_all(&mods_dir).unwrap();
    std::fs::create_dir_all(&scripts_dir).unwrap();
    std::fs::write(
        mods_dir.join("create.jar"),
        zip_bytes(&[(
            "data/create/recipes/materials/andesite_alloy.json",
            br#"{
                "type": "minecraft:crafting_shaped",
                "pattern": ["AB", "BA"],
                "key": {
                    "A": { "item": "minecraft:andesite" },
                    "B": { "tag": "forge:nuggets/iron" }
                },
                "result": { "item": "create:andesite_alloy" }
            }"#,
        )]),
    )
    .unwrap();
    std::fs::write(
        scripts_dir.join("recipes.js"),
        r#"ServerEvents.recipes(event => {
            event.remove({ output: 'create:andesite_alloy' })
            event.custom({
                "type": "minecraft:crafting_shapeless",
                "ingredients": [{ "item": "minecraft:andesite" }, { "item": "minecraft:iron_nugget" }],
                "result": { "item": "create:andesite_alloy", "count": 2 }
            })
        })"#,
    )
    .unwrap();

    let override_hits = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "andesite alloy".to_string(),
        top_k: Some(5),
        kind: Some("recipe_override".to_string()),
        target_id: Some("create:andesite_alloy".to_string()),
        ingredient_id: None,
        include_structured: Some(true),
    })
    .await
    .unwrap();

    let override_hit = override_hits
        .hits
        .first()
        .expect("KubeJS remove should be indexed as a recipe override");
    assert_eq!(override_hit.kind.as_deref(), Some("recipe_override"));
    assert_eq!(
        override_hit
            .structured
            .as_ref()
            .and_then(|value| value.pointer("/action"))
            .and_then(|value| value.as_str()),
        Some("remove")
    );

    let recipe_hits = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "andesite alloy".to_string(),
        top_k: Some(5),
        kind: Some("recipe".to_string()),
        target_id: Some("create:andesite_alloy".to_string()),
        ingredient_id: None,
        include_structured: Some(true),
    })
    .await
    .unwrap();

    let first_recipe = recipe_hits
        .hits
        .first()
        .expect("KubeJS custom recipe should be indexed as a recipe");
    assert_eq!(
        first_recipe
            .structured
            .as_ref()
            .and_then(|value| value.pointer("/source/type"))
            .and_then(|value| value.as_str()),
        Some("kubejs")
    );
    assert_eq!(
        first_recipe
            .structured
            .as_ref()
            .and_then(|value| value.pointer("/result/count"))
            .and_then(|value| value.as_u64()),
        Some(2)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_indexes_kubejs_custom_recipes_with_js_object_syntax() {
    let dir = temp_dir("wiki-kubejs-js-object-recipes");
    let scripts_dir = dir.join("kubejs").join("server_scripts");
    std::fs::create_dir_all(&scripts_dir).unwrap();
    std::fs::write(
        scripts_dir.join("mixing.js"),
        r#"ServerEvents.recipes(event => {
            event.custom({
                type: 'create:mixing',
                ingredients: [
                    { item: 'minecraft:andesite' },
                    { tag: 'forge:nuggets/iron' }
                ],
                results: [{ item: 'create:andesite_alloy', count: 4 }]
            })
        })"#,
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "andesite alloy".to_string(),
        top_k: Some(5),
        kind: Some("recipe".to_string()),
        target_id: Some("create:andesite_alloy".to_string()),
        ingredient_id: Some("#forge:nuggets/iron".to_string()),
        include_structured: Some(true),
    })
    .await
    .unwrap();

    let hit = out
        .hits
        .first()
        .expect("KubeJS custom recipe object syntax should be indexed");
    assert_eq!(hit.kind.as_deref(), Some("recipe"));
    assert_eq!(
        hit.structured
            .as_ref()
            .and_then(|value| value.pointer("/source/type"))
            .and_then(|value| value.as_str()),
        Some("kubejs")
    );
    assert_eq!(
        hit.structured
            .as_ref()
            .and_then(|value| value.pointer("/result/count"))
            .and_then(|value| value.as_u64()),
        Some(4)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_does_not_let_large_lang_files_hide_mod_jar_recipes() {
    let dir = temp_dir("wiki-mod-jar-lang-budget");
    let mods_dir = dir.join("mods");
    std::fs::create_dir_all(&mods_dir).unwrap();
    let huge_lang = format!(r#"{{"item.create.filler":"{}"}}"#, "x".repeat(240 * 1024));
    let mut entries: Vec<(String, Vec<u8>)> = (0..10)
        .map(|idx| {
            (
                format!("assets/create/lang/filler_{idx}.json"),
                huge_lang.as_bytes().to_vec(),
            )
        })
        .collect();
    entries.push((
        "assets/create/lang/en_us.json".to_string(),
        br#"{"item.create.andesite_alloy":"Andesite Alloy"}"#.to_vec(),
    ));
    entries.push((
        "data/create/recipes/crafting/materials/andesite_alloy.json".to_string(),
        br#"{
            "type": "minecraft:crafting_shaped",
            "pattern": ["BA", "AB"],
            "key": {
                "A": { "item": "minecraft:andesite" },
                "B": { "tag": "forge:nuggets/iron" }
            },
            "result": { "item": "create:andesite_alloy" }
        }"#
        .to_vec(),
    ));
    let refs = entries
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect::<Vec<_>>();
    std::fs::write(mods_dir.join("create.jar"), zip_bytes(&refs)).unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "andesite alloy recipe".to_string(),
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
        .find(|hit| {
            hit.kind.as_deref() == Some("recipe")
                && hit
                    .structured
                    .as_ref()
                    .and_then(|value| value.pointer("/result/id"))
                    .and_then(|value| value.as_str())
                    == Some("create:andesite_alloy")
        })
        .expect("recipe entries must be indexed even when jar has many large lang files first");
    assert!(hit.title.contains("Andesite Alloy"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_keeps_large_tags_searchable_without_huge_structured_payloads() {
    let dir = temp_dir("wiki-large-tag-payload");
    let mods_dir = dir.join("mods");
    std::fs::create_dir_all(&mods_dir).unwrap();
    let values = (0..300)
        .map(|idx| format!(r#""example:item_{idx}""#))
        .collect::<Vec<_>>()
        .join(",");
    let tag_json = format!(r#"{{"replace":false,"values":[{values}]}}"#);
    std::fs::write(
        mods_dir.join("big-tags.jar"),
        zip_bytes(&[(
            "data/minecraft/tags/blocks/mineable/pickaxe.json",
            tag_json.as_bytes(),
        )]),
    )
    .unwrap();

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "tag-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "example:item_299".to_string(),
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
        .find(|hit| hit.kind.as_deref() == Some("tag"))
        .expect("large tag values should stay searchable");
    let structured = hit
        .structured
        .as_ref()
        .expect("tag hit has structured data");
    assert_eq!(
        structured
            .get("value_count")
            .and_then(|value| value.as_u64()),
        Some(300)
    );
    assert_eq!(
        structured
            .get("values_truncated")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert!(
        structured
            .get("values")
            .and_then(|value| value.as_array())
            .map(|values| values.len())
            .unwrap_or(usize::MAX)
            <= 128
    );
    assert!(
        serde_json::to_string(structured).unwrap().len() < 16 * 1024,
        "large tag structured payload should remain compact"
    );

    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "tag-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: hit.chunk_id.clone(),
    })
    .await
    .unwrap();
    assert!(opened.chunk.content.contains("example:item_299"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_search_includes_cached_modpack_project_details() {
    let dir = temp_dir("wiki-project-docs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("instance.json"),
        r#"{
            "source": {
                "provider": "modrinth",
                "project_id": "create-above-and-beyond",
                "version_id": "v1"
            }
        }"#,
    )
    .unwrap();
    write_cached_wiki_project_detail(
        &dir,
        "modrinth",
        "create-above-and-beyond",
        "Create: Above and Beyond",
        "This pack has chapter-based automation and kinetic progression.",
    );

    let out = tool_wiki_search(WikiSearchArgs {
        modpack_id: "create-above-and-beyond".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "kinetic progression".to_string(),
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
        .find(|hit| hit.kind.as_deref() == Some("project_doc"))
        .expect("cached project detail should be indexed as a project doc");
    assert_eq!(hit.source_label, "generated:project-doc");
    assert_eq!(
        hit.structured
            .as_ref()
            .and_then(|value| value.pointer("/provider"))
            .and_then(|value| value.as_str()),
        Some("modrinth")
    );

    let opened = tool_wiki_open(WikiOpenArgs {
        modpack_id: "create-above-and-beyond".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        chunk_id: hit.chunk_id.clone(),
    })
    .await
    .unwrap();
    assert!(opened.chunk.content.contains("kinetic progression"));
    assert_eq!(opened.chunk.kind.as_deref(), Some("project_doc"));

    let _ = std::fs::remove_dir_all(&dir);
}
