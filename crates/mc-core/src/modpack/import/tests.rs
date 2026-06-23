//! 导入核心的纯逻辑单测(无网络):
//! - 分发优先级:用假 [`ArchiveIndex`] 验证 confidence 取最高、平局按注册序、早停语义。
//! - modrinth `plan()`:对着 fixture `modrinth.index.json` 验证字段映射、env 过滤、多源。
//! - path-safety:`safe_join` 拒绝 `../` 越权(引擎与 archive 共用同一闸)。

use std::path::Path;

use mc_types::LoaderKind;

use crate::download::Downloader;
use crate::modplatform::provider::ProviderRegistry;

use super::archive::StagingDir;
use super::engine::ImportEngine;
use super::modrinth::{plan_from_index, ModrinthImporter, CLIENT_OVERRIDES, OVERRIDES};
use super::{ArchiveIndex, DetectMatch, ImportPlan, ModpackImporter};

// ===========================================================================
// 假 ArchiveIndex —— 让 detect/dispatch 脱离真实 zip
// ===========================================================================

/// 内存假归档:给一组条目路径,可选给若干条目的内容(供内容判别)。
struct FakeArchive {
    entries: Vec<String>,
    contents: Vec<(String, Vec<u8>)>,
}

impl FakeArchive {
    fn new(entries: &[&str]) -> Self {
        FakeArchive {
            entries: entries.iter().map(|s| s.to_string()).collect(),
            contents: Vec::new(),
        }
    }

    fn with_content(mut self, name: &str, body: &[u8]) -> Self {
        self.contents.push((name.to_string(), body.to_vec()));
        self
    }
}

impl ArchiveIndex for FakeArchive {
    fn entries(&self) -> &[String] {
        &self.entries
    }
    fn read_small(&self, name: &str) -> Option<Vec<u8>> {
        self.contents.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }
}

// ===========================================================================
// 假 importer —— 验证分发优先级而不依赖真实格式
// ===========================================================================

/// 一个命中固定标记并报告固定置信度的假 importer。
struct MarkerImporter {
    id: &'static str,
    marker: &'static str,
}

impl ModpackImporter for MarkerImporter {
    fn id(&self) -> &'static str {
        self.id
    }
    fn detect(&self, archive: &dyn ArchiveIndex) -> Option<DetectMatch> {
        let hit = archive.entries().iter().find(|e| e.ends_with(self.marker))?;
        Some(DetectMatch::from_marker(self.id, hit))
    }
    fn plan(&self, _staging: &Path, _m: &DetectMatch) -> crate::error::Result<ImportPlan> {
        Ok(ImportPlan::new("fake", "1.0"))
    }
}

fn test_engine_with(importers: Vec<Box<dyn ModpackImporter>>) -> ImportEngine {
    let dl = Downloader::new(1).unwrap();
    let mut engine = ImportEngine::new(dl, ProviderRegistry::new());
    for imp in importers {
        engine.register(imp);
    }
    engine
}

// ===========================================================================
// 分发优先级
// ===========================================================================

#[test]
fn dispatch_picks_highest_confidence_marker() {
    // 两个 importer 都命中,但 a 的标记在根级(高 confidence),b 的在深层目录。
    let engine = test_engine_with(vec![
        Box::new(MarkerImporter { id: "a", marker: "a.marker" }),
        Box::new(MarkerImporter { id: "b", marker: "b.marker" }),
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
        Box::new(MarkerImporter { id: "first", marker: "x.marker" }),
        Box::new(MarkerImporter { id: "second", marker: "y.marker" }),
    ]);
    let archive = FakeArchive::new(&["x.marker", "y.marker"]);
    let (idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(idx, 0, "平局应保留先注册者");
    assert_eq!(m.format, "first");
}

#[test]
fn dispatch_returns_none_when_nothing_matches() {
    let engine = test_engine_with(vec![Box::new(MarkerImporter { id: "a", marker: "a.marker" })]);
    let archive = FakeArchive::new(&["README.md", "mods/x.jar"]);
    assert!(engine.dispatch(&archive).is_none());
}

#[test]
fn dispatch_deeper_marker_loses_to_root_even_if_registered_first() {
    // 即便深层标记的 importer 先注册,根级命中的也应凭更高 confidence 胜出。
    let engine = test_engine_with(vec![
        Box::new(MarkerImporter { id: "deep", marker: "deep.marker" }),
        Box::new(MarkerImporter { id: "root", marker: "root.marker" }),
    ]);
    let archive = FakeArchive::new(&["a/b/c/deep.marker", "root.marker"]);
    let (idx, m) = engine.dispatch(&archive).unwrap();
    assert_eq!(idx, 1);
    assert_eq!(m.format, "root");
}

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
    let archive = FakeArchive::new(&[
        "modrinth.index.json",
        "overrides/some/modrinth.index.json",
    ]);
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
    assert_eq!(plan.loader, Some((LoaderKind::Fabric, "0.15.7".to_string())));
    assert_eq!(plan.override_roots, vec![OVERRIDES.to_string(), CLIENT_OVERRIDES.to_string()]);
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

    let sodium = plan.files.iter().find(|f| f.rel_path == "mods/sodium.jar").unwrap();
    // 多源:downloads[] 原样保留为有序候选。
    assert_eq!(sodium.sources.len(), 2);
    assert_eq!(sodium.sources[0], "https://cdn.modrinth.com/data/x/sodium.jar");
    assert_eq!(sodium.sources[1], "https://github.com/CaffeineMC/sodium/releases/sodium.jar");
    assert_eq!(sodium.sha512.as_deref(), Some("longhash"));
    assert_eq!(sodium.sha1.as_deref(), Some("deadbeef"));
    assert_eq!(sodium.size, Some(123456));
    assert!(sodium.required, "client required → 必备");

    // 没有 server-only。
    assert!(plan.files.iter().all(|f| f.rel_path != "mods/server-only.jar"));
}

#[test]
fn modrinth_plan_optional_is_marked_not_required() {
    let plan = fixture_plan();
    let opt = plan.files.iter().find(|f| f.rel_path == "resourcepacks/opt.zip").unwrap();
    assert!(!opt.required, "client optional → 非必备(可跳过)");
}

#[test]
fn modrinth_plan_only_sha512_file_has_no_sha1() {
    let plan = fixture_plan();
    let cfg = plan.files.iter().find(|f| f.rel_path == "config/only512.toml").unwrap();
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
    assert_eq!(plan.loader, Some((LoaderKind::NeoForge, "20.4.237".to_string())));
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

// ===========================================================================
// path-safety —— safe_join 拒绝越权(引擎下文件与 archive 解压共用同一闸)
// ===========================================================================

#[test]
fn safe_join_rejects_parent_traversal() {
    let game_dir = Path::new("/games/mc/versions/pack");
    // 正常相对路径放行。
    assert_eq!(
        crate::fs::safe_join(game_dir, "mods/sodium.jar"),
        Some(std::path::PathBuf::from("/games/mc/versions/pack/mods/sodium.jar"))
    );
    // ../ 越权被拒。
    assert!(crate::fs::safe_join(game_dir, "../../../etc/passwd").is_none());
    assert!(crate::fs::safe_join(game_dir, "../sibling/evil.jar").is_none());
}

#[test]
fn with_defaults_registers_modrinth() {
    let dl = Downloader::new(1).unwrap();
    let engine = ImportEngine::with_defaults(dl, ProviderRegistry::new());
    assert!(engine.importer_count() >= 1, "with_defaults 应至少注册 modrinth");
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
// CurseForge importer —— detect(内容判别)+ plan(fixture manifest,纯)
// ===========================================================================

use super::curseforge::{plan_from_manifest, CurseForgeImporter};
use super::mcbbs::{plan_from_packmeta, McbbsImporter};
use super::multimc::{plan_from_parts, MultiMcImporter};

/// 一份典型 CurseForge `manifest.json`(无 addons/launchInfo)。
const CF_MANIFEST: &str = r#"{
    "manifestType": "minecraftModpack",
    "manifestVersion": 1,
    "name": "All The Mods 9",
    "version": "0.2.60",
    "author": "ATM Team",
    "overrides": "overrides",
    "minecraft": {
        "version": "1.20.1",
        "modLoaders": [ { "id": "neoforge-47.1.0", "primary": true } ],
        "recommendedRam": 8192
    },
    "files": [
        { "projectID": 238222, "fileID": 4567890 },
        { "projectID": 12345, "fileID": 999, "required": false }
    ]
}"#;

#[test]
fn curseforge_detects_manifest_without_addons() {
    let importer = CurseForgeImporter;
    let archive =
        FakeArchive::new(&["manifest.json", "overrides/x"]).with_content("manifest.json", CF_MANIFEST.as_bytes());
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
    assert!(importer.detect(&archive).is_none(), "有 addons 的 manifest 不应被 CurseForge 命中");
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
    assert_eq!(plan.loader, Some((LoaderKind::NeoForge, "47.1.0".to_string())));
    assert_eq!(plan.recommended_ram_mib, Some(8192));
    assert_eq!(plan.override_roots, vec!["overrides".to_string()]);

    // files[] 只给 id → 进 unresolved(待 resolve),plan.files 此刻为空。
    assert!(plan.files.is_empty(), "CF plan() 不带 URL,文件全在 unresolved");
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
    assert!(plan_from_manifest(&manifest).is_err(), "manifestType 不符应拒");
}

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
    assert_eq!(plan.loader, Some((LoaderKind::Fabric, "0.15.7".to_string())));
    // OverrideMemory=true → 6144 进推荐内存。
    assert_eq!(plan.recommended_ram_mib, Some(6144));
    // 游戏目录两种都作 override 根。
    assert_eq!(plan.override_roots, vec![".minecraft".to_string(), "minecraft".to_string()]);
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
    assert!(plan_from_parts(None, &cfg).is_err(), "无 mmc-pack.json 应报错");
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
    let archive = FakeArchive::new(&["manifest.json"])
        .with_content("manifest.json", CF_MANIFEST.as_bytes());
    assert!(importer.detect(&archive).is_none(), "纯 CF manifest 不应被 MCBBS 命中");
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
        vec!["-XX:+UseG1GC".to_string(), "-Dfile.encoding=UTF-8".to_string()]
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
    let meta: crate::modpack::formats::mcbbs::McbbsPackMeta =
        serde_json::from_str(json).unwrap();
    let plan = plan_from_packmeta(&meta).unwrap();
    assert_eq!(plan.loader, Some((LoaderKind::NeoForge, "20.4.190".to_string())));
}

#[test]
fn mcbbs_plan_missing_game_addon_errors() {
    // 无 addons[id=="game"] → 拿不到 MC 版本 → 报错。
    let json = r#"{ "name": "bad", "addons": [ { "id": "forge", "version": "47.0.0" } ] }"#;
    let meta: crate::modpack::formats::mcbbs::McbbsPackMeta =
        serde_json::from_str(json).unwrap();
    assert!(plan_from_packmeta(&meta).is_err());
}

// ===========================================================================
// with_defaults 全注册表分发优先级(含 CF vs MCBBS 内容判别)
// ===========================================================================

fn default_engine() -> ImportEngine {
    let dl = Downloader::new(1).unwrap();
    ImportEngine::with_defaults(dl, ProviderRegistry::new())
}

#[test]
fn with_defaults_registers_all_four_importers() {
    let engine = default_engine();
    assert_eq!(engine.importer_count(), 4, "mcbbs/multimc/modrinth/curseforge");
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
    assert_eq!(m.format, "mcbbs", "有 addons 的 manifest 应判给 MCBBS 而非 CurseForge");
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
