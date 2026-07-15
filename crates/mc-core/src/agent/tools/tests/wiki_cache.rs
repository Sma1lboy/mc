use super::*;

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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    })
    .await
    .unwrap();

    assert_eq!(out.hits.len(), 1);
    assert_eq!(out.hits[0].title, "Cached wiki chunk");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn wiki_refresh_rebuilds_stale_corpus_cache_in_instance_dir() {
    let dir = temp_dir("wiki-refresh-cache");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("guide.md"), "The old guide mentions copper.").unwrap();

    prebuild_wiki_corpus_cache(
        "refresh-pack".to_string(),
        Some("local-instance".to_string()),
        &dir,
    )
    .await
    .unwrap();
    rewrite_first_cached_wiki_chunk(&dir, "stale sentinel answer from old cache");
    std::fs::write(dir.join("guide.md"), "The fresh guide mentions sapphire.").unwrap();

    refresh_wiki_corpus_cache(
        "refresh-pack".to_string(),
        Some("local-instance".to_string()),
        &dir,
    )
    .await
    .unwrap();

    let fresh = tool_wiki_search(WikiSearchArgs {
        modpack_id: "refresh-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "sapphire".to_string(),
        top_k: Some(5),
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    })
    .await
    .unwrap();
    assert_eq!(fresh.hits.len(), 1);

    let stale = tool_wiki_search(WikiSearchArgs {
        modpack_id: "refresh-pack".to_string(),
        instance_id: Some("local-instance".to_string()),
        source_paths: vec![dir.to_string_lossy().to_string()],
        query: "stale sentinel".to_string(),
        top_k: Some(5),
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
    })
    .await
    .unwrap();
    assert!(
        stale.hits.is_empty(),
        "manual refresh must replace stale wiki-corpus.json contents"
    );

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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
        kind: None,
        target_id: None,
        ingredient_id: None,
        include_structured: None,
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
