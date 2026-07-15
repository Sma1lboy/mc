use super::*;

// ===========================================================================
// ModrinthImporter::detect —— 真实 importer 走假归档
// ===========================================================================

#[test]
fn modrinth_detects_root_index() {
    let importer = ModrinthImporter;
    let archive = FakeArchive::new(&["modrinth.index.json", "overrides/mods/a.jar"]);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.format, "modrinth");
    assert_eq!(m.archive_root, "");
    assert_eq!(m.confidence, 1000);
}

#[test]
fn modrinth_detects_nested_index_and_reports_root() {
    let importer = ModrinthImporter;
    let archive = FakeArchive::new(&["MyPack/modrinth.index.json", "MyPack/overrides/x"]);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.archive_root, "MyPack");
    assert!(m.confidence < 1000);
}

#[test]
fn modrinth_picks_shallowest_index_marker() {
    // overrides 内混了同名 index;应取根最浅的标记作为包根(防误判)。
    let importer = ModrinthImporter;
    let archive = FakeArchive::new(&["modrinth.index.json", "overrides/some/modrinth.index.json"]);
    let m = importer.detect(&archive).unwrap();
    assert_eq!(m.archive_root, "");
}

#[test]
fn modrinth_no_marker_is_none() {
    let importer = ModrinthImporter;
    let archive = FakeArchive::new(&["manifest.json", "overrides/x"]);
    assert!(importer.detect(&archive).is_none());
}

// ===========================================================================
// ModrinthImporter::plan —— fixture modrinth.index.json(纯,无网络)
// ===========================================================================

/// 一份覆盖关键分支的 fixture:fabric loader、多源文件、optional 降级、unsupported 跳过、
/// 仅 sha512 文件。
const FIXTURE_INDEX: &str = r#"{
    "formatVersion": 1,
    "game": "minecraft",
    "name": "Fixture Pack",
    "versionId": "2.3.1",
    "summary": "a fixture",
    "dependencies": {
        "minecraft": "1.20.1",
        "fabric-loader": "0.15.7"
    },
    "files": [
        {
            "path": "mods/sodium.jar",
            "downloads": [
                "https://cdn.modrinth.com/data/x/sodium.jar",
                "https://github.com/CaffeineMC/sodium/releases/sodium.jar"
            ],
            "hashes": { "sha512": "longhash", "sha1": "deadbeef" },
            "fileSize": 123456,
            "env": { "client": "required", "server": "optional" }
        },
        {
            "path": "mods/server-only.jar",
            "downloads": ["https://cdn.modrinth.com/data/s/server-only.jar"],
            "hashes": { "sha512": "h2" },
            "env": { "client": "unsupported", "server": "required" }
        },
        {
            "path": "resourcepacks/opt.zip",
            "downloads": ["https://cdn.modrinth.com/data/o/opt.zip"],
            "hashes": { "sha512": "h3" },
            "env": { "client": "optional", "server": "unsupported" }
        },
        {
            "path": "config/only512.toml",
            "downloads": ["https://cdn.modrinth.com/data/c/cfg.toml"],
            "hashes": { "sha512": "onlybig" }
        }
    ]
}"#;

fn fixture_plan() -> ImportPlan {
    let index: crate::modpack::formats::mrpack::MrpackIndex =
        serde_json::from_str(FIXTURE_INDEX).unwrap();
    plan_from_index(&index).unwrap()
}

#[test]
fn modrinth_plan_maps_metadata_and_loader() {
    let plan = fixture_plan();
    assert_eq!(plan.pack_name, "Fixture Pack");
    assert_eq!(plan.pack_version.as_deref(), Some("2.3.1"));
    assert_eq!(plan.mc_version, "1.20.1");
    assert_eq!(
        plan.loader,
        Some((LoaderKind::Fabric, "0.15.7".to_string()))
    );
    assert_eq!(
        plan.override_roots,
        vec![OVERRIDES.to_string(), CLIENT_OVERRIDES.to_string()]
    );
    // 溯源记录到 modrinth。
    let managed = plan.managed.as_ref().unwrap();
    assert_eq!(managed.platform, "modrinth");
    assert_eq!(managed.version_id.as_deref(), Some("2.3.1"));
}

#[test]
fn modrinth_plan_filters_unsupported_and_keeps_multisource() {
    let plan = fixture_plan();
    // server-only(client unsupported)被过滤;其余 3 个保留。
    assert_eq!(plan.files.len(), 3, "client-unsupported 文件应被跳过");

    let sodium = plan
        .files
        .iter()
        .find(|f| f.rel_path == "mods/sodium.jar")
        .unwrap();
    // 多源:downloads[] 原样保留为有序候选。
    assert_eq!(sodium.sources.len(), 2);
    assert_eq!(
        sodium.sources[0],
        "https://cdn.modrinth.com/data/x/sodium.jar"
    );
    assert_eq!(
        sodium.sources[1],
        "https://github.com/CaffeineMC/sodium/releases/sodium.jar"
    );
    assert_eq!(sodium.sha512.as_deref(), Some("longhash"));
    assert_eq!(sodium.sha1.as_deref(), Some("deadbeef"));
    assert_eq!(sodium.size, Some(123456));
    assert!(sodium.required, "client required → 必备");

    // 没有 server-only。
    assert!(plan
        .files
        .iter()
        .all(|f| f.rel_path != "mods/server-only.jar"));
}

#[test]
fn modrinth_plan_optional_is_marked_not_required() {
    let plan = fixture_plan();
    let opt = plan
        .files
        .iter()
        .find(|f| f.rel_path == "resourcepacks/opt.zip")
        .unwrap();
    assert!(!opt.required, "client optional → 非必备(可跳过)");
}

#[test]
fn modrinth_plan_only_sha512_file_has_no_sha1() {
    let plan = fixture_plan();
    let cfg = plan
        .files
        .iter()
        .find(|f| f.rel_path == "config/only512.toml")
        .unwrap();
    assert_eq!(cfg.sha512.as_deref(), Some("onlybig"));
    assert!(cfg.sha1.is_none());
    assert!(cfg.required, "无 env → 缺省必备");
}

#[test]
fn modrinth_plan_vanilla_pack_has_no_loader() {
    let index_json = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "name": "Vanilla Pack",
        "dependencies": { "minecraft": "1.21" }
    }"#;
    let index: crate::modpack::formats::mrpack::MrpackIndex =
        serde_json::from_str(index_json).unwrap();
    let plan = plan_from_index(&index).unwrap();
    assert_eq!(plan.mc_version, "1.21");
    assert!(plan.loader.is_none());
    assert!(plan.files.is_empty());
}

#[test]
fn modrinth_plan_neoforge_dependency_maps_to_neoforge() {
    let index_json = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "name": "Neo Pack",
        "dependencies": { "minecraft": "1.20.4", "neoforge": "20.4.237" }
    }"#;
    let index: crate::modpack::formats::mrpack::MrpackIndex =
        serde_json::from_str(index_json).unwrap();
    let plan = plan_from_index(&index).unwrap();
    assert_eq!(
        plan.loader,
        Some((LoaderKind::NeoForge, "20.4.237".to_string()))
    );
}

#[test]
fn modrinth_plan_missing_minecraft_errors() {
    // dependencies 缺 minecraft → plan 报错(而非静默)。
    let index_json = r#"{
        "formatVersion": 1,
        "game": "minecraft",
        "name": "Bad",
        "dependencies": {}
    }"#;
    let index: crate::modpack::formats::mrpack::MrpackIndex =
        serde_json::from_str(index_json).unwrap();
    assert!(plan_from_index(&index).is_err());
}

// ===========================================================================
// ModrinthImporter::plan 从 staging 落盘读(端到端解析路径)
// ===========================================================================

#[test]
fn modrinth_plan_reads_index_from_staging() {
    let staging = StagingDir::new().unwrap();
    std::fs::write(staging.path().join("modrinth.index.json"), FIXTURE_INDEX).unwrap();
    let importer = ModrinthImporter;
    let det = DetectMatch::from_marker("modrinth", "modrinth.index.json");
    let plan = importer.plan(staging.path(), &det).unwrap();
    assert_eq!(plan.mc_version, "1.20.1");
    assert_eq!(plan.files.len(), 3);
}
