use super::*;

// ===========================================================================
// MultiMC importer —— detect(嵌套根)+ plan(fixture mmc-pack + instance.cfg,纯)
// ===========================================================================

#[test]
fn multimc_detects_mmc_pack_at_root() {
    let importer = MultiMcImporter;
    let archive = FakeArchive::new(&["mmc-pack.json", "instance.cfg", ".minecraft/mods/a.jar"]);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.format, "multimc");
    assert_eq!(m.archive_root, "");
}

#[test]
fn multimc_detects_only_instance_cfg() {
    let importer = MultiMcImporter;
    let archive = FakeArchive::new(&["instance.cfg", "minecraft/options.txt"]);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.format, "multimc");
}

#[test]
fn multimc_captures_nested_root() {
    let importer = MultiMcImporter;
    let archive = FakeArchive::new(&["MyInstance/mmc-pack.json", "MyInstance/.minecraft/x"]);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.archive_root, "MyInstance");
    assert!(m.confidence < 1000);
}

#[test]
fn multimc_no_marker_is_none() {
    let importer = MultiMcImporter;
    let archive = FakeArchive::new(&["modrinth.index.json"]);
    assert!(importer.detect(&archive).is_none());
}

fn parse_mmc(json: &str) -> crate::modpack::formats::multimc::MmcPack {
    serde_json::from_str(json).unwrap()
}

#[test]
fn multimc_plan_from_pack_and_cfg() {
    let pack = parse_mmc(
        r#"{
        "formatVersion": 1,
        "components": [
            { "uid": "net.minecraft", "version": "1.20.1", "important": true },
            { "uid": "org.lwjgl3", "version": "3.3.1", "dependencyOnly": true },
            { "uid": "net.fabricmc.fabric-loader", "version": "0.15.7", "important": true }
        ]
    }"#,
    );
    let cfg_text = "\
name=My Prism Pack
OverrideMemory=true
MaxMemAlloc=6144
ManagedPackType=modrinth
ManagedPackID=AABBCCDD
ManagedPackVersionID=v7
";
    let cfg = crate::modpack::formats::multimc::parse_instance_cfg(cfg_text);
    let plan = plan_from_parts(Some(&pack), &cfg).unwrap();

    assert_eq!(plan.pack_name, "My Prism Pack");
    assert_eq!(plan.mc_version, "1.20.1");
    assert_eq!(
        plan.loader,
        Some((LoaderKind::Fabric, "0.15.7".to_string()))
    );
    // OverrideMemory=true → 6144 进推荐内存。
    assert_eq!(plan.recommended_ram_mib, Some(6144));
    // 游戏目录两种都作 override 根。
    assert_eq!(
        plan.override_roots,
        vec![".minecraft".to_string(), "minecraft".to_string()]
    );
    // 无远程文件。
    assert!(plan.files.is_empty());
    assert!(plan.unresolved.is_empty());
    // 溯源优先用 instance.cfg 的 ManagedPack*。
    let managed = plan.managed.as_ref().unwrap();
    assert_eq!(managed.platform, "modrinth");
    assert_eq!(managed.project_id, "AABBCCDD");
    assert_eq!(managed.version_id.as_deref(), Some("v7"));
}

#[test]
fn multimc_plan_vanilla_instance_has_no_loader() {
    let pack = parse_mmc(
        r#"{ "formatVersion": 1, "components": [
            { "uid": "net.minecraft", "version": "1.21", "important": true }
        ]}"#,
    );
    let cfg = crate::modpack::formats::multimc::InstanceCfg::default();
    let plan = plan_from_parts(Some(&pack), &cfg).unwrap();
    assert_eq!(plan.mc_version, "1.21");
    assert!(plan.loader.is_none());
    // 无 instance.cfg 名字 → 兜底名。
    assert_eq!(plan.pack_name, "MultiMC Instance");
    // 无 ManagedPack* → 溯源记到 multimc 自身。
    assert_eq!(plan.managed.as_ref().unwrap().platform, "multimc");
}

#[test]
fn multimc_plan_missing_pack_errors() {
    let cfg = crate::modpack::formats::multimc::InstanceCfg::default();
    assert!(
        plan_from_parts(None, &cfg).is_err(),
        "无 mmc-pack.json 应报错"
    );
}

#[test]
fn multimc_plan_memory_gate_off_no_ram() {
    let pack = parse_mmc(
        r#"{ "formatVersion": 1, "components": [
            { "uid": "net.minecraft", "version": "1.20.1" }
        ]}"#,
    );
    // 没有 OverrideMemory=true → MaxMemAlloc 不进 typed → 不设推荐内存。
    let cfg = crate::modpack::formats::multimc::parse_instance_cfg("name=X\nMaxMemAlloc=4096\n");
    let plan = plan_from_parts(Some(&pack), &cfg).unwrap();
    assert_eq!(plan.recommended_ram_mib, None);
}
