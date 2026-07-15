use super::*;

// ===========================================================================
// CurseForge importer —— detect(内容判别)+ plan(fixture manifest,纯)
// ===========================================================================

#[test]
fn curseforge_detects_manifest_without_addons() {
    let importer = CurseForgeImporter;
    let archive = FakeArchive::new(&["manifest.json", "overrides/x"])
        .with_content("manifest.json", CF_MANIFEST.as_bytes());
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.format, "curseforge");
    assert_eq!(m.archive_root, "");
    assert_eq!(m.confidence, 1000);
}

#[test]
fn curseforge_does_not_detect_mcbbs_manifest() {
    // 同名 manifest.json 但有 addons → 不是 CurseForge(归 MCBBS)。
    let importer = CurseForgeImporter;
    let body = br#"{ "manifestType": "minecraftModpack", "manifestVersion": 1,
        "addons": [ { "id": "game", "version": "1.20.1" } ] }"#;
    let archive = FakeArchive::new(&["manifest.json"]).with_content("manifest.json", body);
    assert!(
        importer.detect(&archive).is_none(),
        "有 addons 的 manifest 不应被 CurseForge 命中"
    );
}

#[test]
fn curseforge_ignores_manifest_in_overrides_only() {
    // overrides 内的 manifest.json 不是包根标记;取最浅命中后内容判别,这里仅深层有 →
    // 取该深层但内容仍是 CF manifest 时命中(confidence 较低)。验证取最浅。
    let importer = CurseForgeImporter;
    let archive = FakeArchive::new(&["manifest.json", "overrides/some/manifest.json"])
        .with_content("manifest.json", CF_MANIFEST.as_bytes());
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.archive_root, "", "应取根最浅的 manifest.json");
}

#[test]
fn curseforge_plan_maps_metadata_loader_and_unresolved() {
    let manifest: crate::modpack::formats::curseforge::FlameManifest =
        serde_json::from_str(CF_MANIFEST).unwrap();
    let plan = plan_from_manifest(&manifest).unwrap();

    assert_eq!(plan.pack_name, "All The Mods 9");
    assert_eq!(plan.pack_version.as_deref(), Some("0.2.60"));
    assert_eq!(plan.mc_version, "1.20.1");
    assert_eq!(
        plan.loader,
        Some((LoaderKind::NeoForge, "47.1.0".to_string()))
    );
    assert_eq!(plan.recommended_ram_mib, Some(8192));
    assert_eq!(plan.override_roots, vec!["overrides".to_string()]);

    // files[] 只给 id → 进 unresolved(待 resolve),plan.files 此刻为空。
    assert!(
        plan.files.is_empty(),
        "CF plan() 不带 URL,文件全在 unresolved"
    );
    assert_eq!(plan.unresolved.len(), 2);
    let first = &plan.unresolved[0];
    assert_eq!(first.project_id, "238222");
    assert_eq!(first.file_id, "4567890");
    assert_eq!(first.target_dir, "mods");
    assert!(first.required, "未给 required 默认 true");
    assert!(!plan.unresolved[1].required, "显式 required=false");

    let managed = plan.managed.as_ref().unwrap();
    assert_eq!(managed.platform, "curseforge");
}

#[test]
fn curseforge_plan_custom_overrides_dir() {
    let json = r#"{
        "manifestType": "minecraftModpack",
        "manifestVersion": 1,
        "name": "Custom",
        "overrides": "src",
        "minecraft": { "version": "1.20.1", "modLoaders": [ { "id": "forge-47.2.0" } ] }
    }"#;
    let manifest: crate::modpack::formats::curseforge::FlameManifest =
        serde_json::from_str(json).unwrap();
    let plan = plan_from_manifest(&manifest).unwrap();
    assert_eq!(plan.override_roots, vec!["src".to_string()]);
    assert_eq!(plan.loader, Some((LoaderKind::Forge, "47.2.0".to_string())));
}

#[test]
fn curseforge_plan_vanilla_has_no_loader() {
    let json = r#"{
        "manifestType": "minecraftModpack",
        "manifestVersion": 1,
        "name": "Vanilla CF",
        "minecraft": { "version": "1.21" }
    }"#;
    let manifest: crate::modpack::formats::curseforge::FlameManifest =
        serde_json::from_str(json).unwrap();
    let plan = plan_from_manifest(&manifest).unwrap();
    assert!(plan.loader.is_none());
    assert!(plan.unresolved.is_empty());
}

#[test]
fn curseforge_plan_invalid_manifest_type_errors() {
    let json = r#"{
        "manifestType": "somethingElse",
        "manifestVersion": 1,
        "minecraft": { "version": "1.20.1" }
    }"#;
    let manifest: crate::modpack::formats::curseforge::FlameManifest =
        serde_json::from_str(json).unwrap();
    assert!(
        plan_from_manifest(&manifest).is_err(),
        "manifestType 不符应拒"
    );
}
