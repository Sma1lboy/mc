use super::*;

// ===========================================================================
// MCBBS importer —— detect(packmeta 与内容判别)+ plan(fixture packmeta,纯)
// ===========================================================================

#[test]
fn mcbbs_detects_packmeta_marker() {
    let importer = McbbsImporter;
    // packmeta 是唯一命名标记,无需读内容即命中。
    let archive = FakeArchive::new(&["mcbbs.packmeta", "overrides/x"]);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.format, "mcbbs");
    assert_eq!(m.archive_root, "");
    assert_eq!(m.confidence, 1000);
}

#[test]
fn mcbbs_detects_manifest_with_addons() {
    let importer = McbbsImporter;
    let body = br#"{ "manifestType": "minecraftModpack",
        "addons": [ { "id": "game", "version": "1.20.1" } ] }"#;
    let archive = FakeArchive::new(&["manifest.json"]).with_content("manifest.json", body);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.format, "mcbbs");
}

#[test]
fn mcbbs_detects_manifest_with_launch_info() {
    let importer = McbbsImporter;
    let body = br#"{ "manifestType": "minecraftModpack", "launchInfo": { "minMemory": 4096 },
        "addons": [ { "id": "game", "version": "1.19.2" } ] }"#;
    let archive = FakeArchive::new(&["manifest.json"]).with_content("manifest.json", body);
    assert!(importer.detect(&archive).is_some());
}

#[test]
fn mcbbs_does_not_detect_plain_curseforge_manifest() {
    // 无 addons/launchInfo 的 manifest.json 不是 MCBBS(归 CurseForge)。
    let importer = McbbsImporter;
    let archive =
        FakeArchive::new(&["manifest.json"]).with_content("manifest.json", CF_MANIFEST.as_bytes());
    assert!(
        importer.detect(&archive).is_none(),
        "纯 CF manifest 不应被 MCBBS 命中"
    );
}

const MCBBS_PACKMETA: &str = r#"{
    "manifestType": "minecraftModpack",
    "manifestVersion": 1,
    "name": "国服整合包",
    "version": "3.1",
    "author": "someone",
    "fileApi": "https://mirror.example/files",
    "addons": [
        { "id": "game", "version": "1.20.1" },
        { "id": "forge", "version": "47.2.0" }
    ],
    "files": [
        { "projectID": 238222, "fileID": 4567890, "type": "curse" },
        { "projectID": 12345, "fileID": 999, "type": "curse", "required": false }
    ],
    "launchInfo": {
        "minMemory": 4096,
        "supportedJavaVersions": [17, 21],
        "launchArgument": ["--fullscreen"],
        "javaArgument": ["-XX:+UseG1GC", "-Dfile.encoding=UTF-8"]
    }
}"#;

#[test]
fn mcbbs_plan_maps_addons_files_and_launch_info() {
    let meta: crate::modpack::formats::mcbbs::McbbsPackMeta =
        serde_json::from_str(MCBBS_PACKMETA).unwrap();
    let plan = plan_from_packmeta(&meta).unwrap();

    assert_eq!(plan.pack_name, "国服整合包");
    assert_eq!(plan.pack_version.as_deref(), Some("3.1"));
    // addons[id=="game"] → mc 版本;forge addon → loader。
    assert_eq!(plan.mc_version, "1.20.1");
    assert_eq!(plan.loader, Some((LoaderKind::Forge, "47.2.0".to_string())));
    // launchInfo → 实例参数 + 内存。
    assert_eq!(plan.extra_game_args, vec!["--fullscreen".to_string()]);
    assert_eq!(
        plan.extra_jvm_args,
        vec![
            "-XX:+UseG1GC".to_string(),
            "-Dfile.encoding=UTF-8".to_string()
        ]
    );
    assert_eq!(plan.recommended_ram_mib, Some(4096));
    assert_eq!(plan.override_roots, vec!["overrides".to_string()]);

    // CurseForge-shaped files → unresolved(待 CF resolve)。
    assert_eq!(plan.unresolved.len(), 2);
    assert_eq!(plan.unresolved[0].project_id, "238222");
    assert_eq!(plan.unresolved[0].file_id, "4567890");
    assert!(plan.unresolved[0].required);
    assert!(!plan.unresolved[1].required);

    assert_eq!(plan.managed.as_ref().unwrap().platform, "mcbbs");
}

#[test]
fn mcbbs_plan_neoforge_addon() {
    let json = r#"{
        "name": "neo pack",
        "addons": [
            { "id": "game", "version": "1.20.4" },
            { "id": "neoforge", "version": "20.4.190" }
        ]
    }"#;
    let meta: crate::modpack::formats::mcbbs::McbbsPackMeta = serde_json::from_str(json).unwrap();
    let plan = plan_from_packmeta(&meta).unwrap();
    assert_eq!(
        plan.loader,
        Some((LoaderKind::NeoForge, "20.4.190".to_string()))
    );
}

#[test]
fn mcbbs_plan_missing_game_addon_errors() {
    // 无 addons[id=="game"] → 拿不到 MC 版本 → 报错。
    let json = r#"{ "name": "bad", "addons": [ { "id": "forge", "version": "47.0.0" } ] }"#;
    let meta: crate::modpack::formats::mcbbs::McbbsPackMeta = serde_json::from_str(json).unwrap();
    assert!(plan_from_packmeta(&meta).is_err());
}
