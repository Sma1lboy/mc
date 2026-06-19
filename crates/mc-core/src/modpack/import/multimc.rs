//! MultiMC / Prism 原生实例导入器(目录即包)。
//!
//! 不是单文件,而是一个实例**目录**(导出 = 该目录打 zip)。两文件驱动:
//! - `mmc-pack.json`:组件图 → 取 `net.minecraft` 版本与 loader 组件(经
//!   [`crate::modpack::formats::multimc::MmcPack`])。
//! - `instance.cfg`:无 section INI → 实例名 / 内存(经
//!   [`crate::modpack::formats::multimc::parse_instance_cfg`])。
//!
//! 游戏数据是**预装好的 loose 文件**(无远程 `files[]`),整目录铺进游戏目录;MultiMC 把游戏
//! 目录放在 `.minecraft/`(新)或 `minecraft/`(旧),两者都作 override 根(引擎对不存在的根
//! 空操作,故同时列出即"取存在的那个")。
//!
//! `resolve()` 空操作(无 id 引用)。**安全**:`instance.cfg` 的 `JavaPath` / 启动命令不可
//! 自动信任(会执行任意二进制),本 importer **不**把它们写进 plan —— 只取名字与内存。

use std::path::Path;

use crate::error::{CoreError, Result};
use crate::modpack::formats::multimc::{parse_instance_cfg, MmcLoaderKind, MmcPack};

use super::{ArchiveIndex, DetectMatch, ImportPlan, ManagedPack, ModpackImporter};

/// 组件图标记 basename。
const MARK_PACK: &str = "mmc-pack.json";
/// 实例配置标记 basename(无 `mmc-pack.json` 的老实例也认它)。
const MARK_CFG: &str = "instance.cfg";
/// MultiMC 游戏目录(新),作 override 根。
const GAME_DIR_DOT: &str = ".minecraft";
/// MultiMC 游戏目录(旧),作 override 根。
const GAME_DIR_PLAIN: &str = "minecraft";

/// MultiMC / Prism 原生实例导入器。
pub struct MultiMcImporter;

impl ModpackImporter for MultiMcImporter {
    fn id(&self) -> &'static str {
        "multimc"
    }

    fn detect(&self, archive: &dyn ArchiveIndex) -> Option<DetectMatch> {
        // 取 mmc-pack.json / instance.cfg 中根最浅的命中作为标记(捕获嵌套一层目录的包根)。
        let pack = shallowest_marker(archive, MARK_PACK);
        let cfg = shallowest_marker(archive, MARK_CFG);
        let marker = shallower(pack, cfg)?;
        Some(DetectMatch::from_marker(self.id(), &marker))
    }

    fn plan(&self, staging: &Path, _m: &DetectMatch) -> Result<ImportPlan> {
        // mmc-pack.json 是组件图,优先从它取 mc + loader;无则报错(老实例至少要有它定版本)。
        let pack_path = staging.join(MARK_PACK);
        let pack: Option<MmcPack> = match std::fs::read_to_string(&pack_path) {
            Ok(raw) => Some(
                serde_json::from_str(&raw)
                    .map_err(|e| CoreError::Parse { what: MARK_PACK.into(), source: e })?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(CoreError::io(&pack_path, e)),
        };

        // instance.cfg 取名字 / 内存(无 section INI;不存在则空配置)。
        let cfg = match std::fs::read_to_string(staging.join(MARK_CFG)) {
            Ok(text) => parse_instance_cfg(&text),
            Err(_) => Default::default(),
        };

        plan_from_parts(pack.as_ref(), &cfg)
    }
}

/// 纯解析:从(可选)组件图 + 已解析的 `instance.cfg` 产出 [`ImportPlan`]。
///
/// 抽成自由函数便于单测:直接喂解析后的 [`MmcPack`] / [`crate::modpack::formats::multimc::InstanceCfg`],
/// 无需落盘 staging。MC 版本必须能从组件图(`net.minecraft`)拿到,否则报错。
pub(crate) fn plan_from_parts(
    pack: Option<&MmcPack>,
    cfg: &crate::modpack::formats::multimc::InstanceCfg,
) -> Result<ImportPlan> {
    let pack = pack.ok_or_else(|| {
        CoreError::other("MultiMC 实例缺少 mmc-pack.json(无法确定 Minecraft 版本)")
    })?;

    let mc_version = pack
        .minecraft_version()
        .ok_or_else(|| CoreError::other("mmc-pack.json 缺少 net.minecraft 组件版本"))?
        .to_string();

    // 名字:优先 instance.cfg 的 name,其次 ManagedPack 名,最后兜底常量。
    let pack_name = cfg
        .name
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "MultiMC Instance".to_string());

    let mut plan = ImportPlan::new(pack_name, mc_version);

    // loader:组件图里第一个 loader 组件 → (LoaderKind, 版本)。无则原版。
    if let Some((kind, comp)) = pack.loader() {
        let version = comp.version.clone().unwrap_or_default();
        plan.loader = Some((mmc_loader_to_core(kind), version));
    }

    // 内存:instance.cfg 仅在 OverrideMemory=true 时给出 MaxMemAlloc(已由解析器把闸处理好)。
    if let Some(max_mem) = cfg.max_mem_alloc {
        plan.recommended_ram_mib = Some(max_mem as u64);
    }

    // 游戏数据是预装好的 loose 文件:两种游戏目录都作 override 根(存在哪个铺哪个)。
    plan.override_roots = vec![GAME_DIR_DOT.to_string(), GAME_DIR_PLAIN.to_string()];

    // 溯源:优先用 instance.cfg 的 ManagedPack*(这实例本身来自 modrinth/flame/…)。
    if let Some(platform) = cfg.managed_pack_type.clone().filter(|s| !s.is_empty()) {
        plan.managed = Some(ManagedPack {
            platform,
            project_id: cfg.managed_pack_id.clone().unwrap_or_default(),
            version_id: cfg.managed_pack_version_id.clone(),
        });
    } else {
        plan.managed = Some(ManagedPack {
            platform: "multimc".to_string(),
            project_id: String::new(),
            version_id: None,
        });
    }

    Ok(plan)
}

/// 把 [`MmcLoaderKind`] 映射到统一 [`mc_types::LoaderKind`]。
fn mmc_loader_to_core(kind: MmcLoaderKind) -> mc_types::LoaderKind {
    match kind {
        MmcLoaderKind::NeoForge => mc_types::LoaderKind::NeoForge,
        MmcLoaderKind::Forge => mc_types::LoaderKind::Forge,
        MmcLoaderKind::Fabric => mc_types::LoaderKind::Fabric,
        MmcLoaderKind::Quilt => mc_types::LoaderKind::Quilt,
        MmcLoaderKind::LiteLoader => mc_types::LoaderKind::LiteLoader,
    }
}

/// 找 basename 恰为 `name` 的最浅命中条目(`/` 段数最少)。
fn shallowest_marker(archive: &dyn ArchiveIndex, name: &str) -> Option<String> {
    archive
        .entries()
        .iter()
        .filter(|e| basename(e) == name)
        .min_by_key(|e| depth(e))
        .cloned()
}

/// 在两个可选命中里取根更浅者。
fn shallower(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if depth(&x) <= depth(&y) { x } else { y }),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

/// 条目 basename(最后一个 `/` 之后)。
fn basename(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

/// 路径深度(`/` 段数)。
fn depth(entry: &str) -> usize {
    entry.split('/').filter(|s| !s.is_empty()).count()
}
