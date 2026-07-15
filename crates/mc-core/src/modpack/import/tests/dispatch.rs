use super::*;

// ===========================================================================
// 分发优先级
// ===========================================================================

#[test]
fn dispatch_picks_highest_confidence_marker() {
    // 两个 importer 都命中,但 a 的标记在根级(高 confidence),b 的在深层目录。
    let engine = test_engine_with(vec![
        Box::new(MarkerImporter {
            id: "a",
            marker: "a.marker",
        }),
        Box::new(MarkerImporter {
            id: "b",
            marker: "b.marker",
        }),
    ]);
    let archive = FakeArchive::new(&["a.marker", "deep/sub/b.marker"]);
    let (idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(idx, 0);
    assert_eq!(m.format, "a", "根级标记 confidence 更高应胜出");
    assert_eq!(m.archive_root, "");
}

#[test]
fn dispatch_tie_breaks_by_registration_order() {
    // 两个 importer 同样在根级命中(confidence 相同)→ 先注册者(b 先注册)胜。
    let engine = test_engine_with(vec![
        Box::new(MarkerImporter {
            id: "first",
            marker: "x.marker",
        }),
        Box::new(MarkerImporter {
            id: "second",
            marker: "y.marker",
        }),
    ]);
    let archive = FakeArchive::new(&["x.marker", "y.marker"]);
    let (idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(idx, 0, "平局应保留先注册者");
    assert_eq!(m.format, "first");
}

#[test]
fn dispatch_returns_none_when_nothing_matches() {
    let engine = test_engine_with(vec![Box::new(MarkerImporter {
        id: "a",
        marker: "a.marker",
    })]);
    let archive = FakeArchive::new(&["README.md", "mods/x.jar"]);
    assert!(engine.dispatch(&archive).is_none());
}

#[test]
fn dispatch_deeper_marker_loses_to_root_even_if_registered_first() {
    // 即便深层标记的 importer 先注册,根级命中的也应凭更高 confidence 胜出。
    let engine = test_engine_with(vec![
        Box::new(MarkerImporter {
            id: "deep",
            marker: "deep.marker",
        }),
        Box::new(MarkerImporter {
            id: "root",
            marker: "root.marker",
        }),
    ]);
    let archive = FakeArchive::new(&["a/b/c/deep.marker", "root.marker"]);
    let (idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(idx, 1);
    assert_eq!(m.format, "root");
}

// ===========================================================================
// path-safety —— safe_join 拒绝越权(引擎下文件与 archive 解压共用同一闸)
// ===========================================================================

#[test]
fn safe_join_rejects_parent_traversal() {
    let game_dir = Path::new("/games/mc/versions/pack");
    // 正常相对路径放行。
    assert_eq!(
        crate::fs::safe_join(game_dir, "mods/sodium.jar"),
        Some(std::path::PathBuf::from(
            "/games/mc/versions/pack/mods/sodium.jar"
        ))
    );
    // ../ 越权被拒。
    assert!(crate::fs::safe_join(game_dir, "../../../etc/passwd").is_none());
    assert!(crate::fs::safe_join(game_dir, "../sibling/evil.jar").is_none());
}

#[test]
fn with_defaults_registers_modrinth() {
    let dl = Downloader::new(1).unwrap();
    let engine = ImportEngine::with_defaults(dl, ProviderRegistry::new());
    assert!(
        engine.importer_count() >= 1,
        "with_defaults 应至少注册 modrinth"
    );
    // dispatch 一个 mrpack 索引应命中 modrinth。
    let archive = FakeArchive::new(&["modrinth.index.json"]);
    let (_idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(m.format, "modrinth");
}

// 让 with_content 在某测试里被用到,避免 dead_code 警告(内容判别的 fake 路径)。
#[test]
fn fake_archive_read_small_returns_cached_content() {
    let archive = FakeArchive::new(&["manifest.json"]).with_content("manifest.json", b"{}");
    assert_eq!(archive.read_small("manifest.json"), Some(b"{}".to_vec()));
    assert!(archive.read_small("missing").is_none());
}

// ===========================================================================
// with_defaults 全注册表分发优先级(含 CF vs MCBBS 内容判别)
// ===========================================================================

#[test]
fn with_defaults_registers_all_four_importers() {
    let engine = default_engine();
    assert_eq!(
        engine.importer_count(),
        4,
        "mcbbs/multimc/modrinth/curseforge"
    );
}

#[test]
fn dispatch_plain_manifest_goes_to_curseforge() {
    // 无 addons 的 manifest.json:mcbbs detect 返回 None → curseforge 胜出。
    let engine = default_engine();
    let archive = FakeArchive::new(&["manifest.json", "overrides/x"])
        .with_content("manifest.json", CF_MANIFEST.as_bytes());
    let (_idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(m.format, "curseforge");
}

#[test]
fn dispatch_manifest_with_addons_goes_to_mcbbs() {
    // 有 addons 的同名 manifest.json:mcbbs 命中(且注册在前)→ MCBBS 胜出。
    let engine = default_engine();
    let body = br#"{ "manifestType": "minecraftModpack",
        "addons": [ { "id": "game", "version": "1.20.1" } ] }"#;
    let archive = FakeArchive::new(&["manifest.json"]).with_content("manifest.json", body);
    let (_idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(
        m.format, "mcbbs",
        "有 addons 的 manifest 应判给 MCBBS 而非 CurseForge"
    );
}

#[test]
fn dispatch_packmeta_beats_curseforge_manifest() {
    // mcbbs.packmeta 与 manifest.json(纯 CF)并存:packmeta 唯一命名标记 → MCBBS 胜出。
    let engine = default_engine();
    let archive = FakeArchive::new(&["mcbbs.packmeta", "manifest.json", "overrides/x"])
        .with_content("manifest.json", CF_MANIFEST.as_bytes());
    let (_idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(m.format, "mcbbs");
}

#[test]
fn dispatch_mmc_pack_beats_modrinth() {
    // multimc 注册在 modrinth 之前;两标记都在根级(平局)→ 先注册的 multimc 胜出。
    let engine = default_engine();
    let archive = FakeArchive::new(&["mmc-pack.json", "modrinth.index.json"]);
    let (_idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(m.format, "multimc");
}

#[test]
fn dispatch_modrinth_still_wins_for_mrpack() {
    let engine = default_engine();
    let archive = FakeArchive::new(&["modrinth.index.json", "overrides/mods/a.jar"]);
    let (_idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(m.format, "modrinth");
}
