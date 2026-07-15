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
        self.contents
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
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
        let hit = archive
            .entries()
            .iter()
            .find(|e| e.ends_with(self.marker))?;
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

fn default_engine() -> ImportEngine {
    let dl = Downloader::new(1).unwrap();
    ImportEngine::with_defaults(dl, ProviderRegistry::new())
}

mod curseforge;
mod dispatch;
mod mcbbs;
mod modrinth;
mod multimc;
