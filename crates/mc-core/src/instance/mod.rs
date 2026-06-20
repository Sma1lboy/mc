//! 实例管理 —— "版本即实例"模型。
//!
//! 不像 Prism 那样把实例放在独立的 `instances/<name>/.minecraft/` 里,本启动器
//! 直接把每个 game root 下的 `versions/<id>/` 当作一个实例(贴近官启/HMCL/PCL 的
//! 目录布局,见 `docs/07-directory-model-portability.md` 与 `docs/modules/instance.md`)。
//!
//! 一个目录 `versions/<id>/` 被视为实例,当且仅当其中存在版本 json
//! (`versions/<id>/<id>.json`)。实例的可覆盖设置存放在同目录的
//! `instance.json`(见 [`config`])。本模块只负责"枚举/读写已存在的实例目录",
//! 实际的版本/库/资源下载由 launch/meta 层负责。

pub mod config;
pub mod install_mod;
pub mod lifecycle;
pub mod mods;
pub mod packs;
pub mod world;

pub use config::InstanceConfig;
pub use install_mod::{install_mod, install_mod_version, InstallReport};
pub use mods::{list_mods, ModInfo};
pub use packs::{list_packs, PackKind};
pub use world::{list_worlds, WorldInfo};

use std::path::{Path, PathBuf};

use mc_types::{InstanceSummary, LoaderKind};

use crate::error::Result;
use crate::paths::GamePaths;

/// `instance.json` 在实例目录下的固定文件名。
const INSTANCE_CONFIG_FILE: &str = "instance.json";

/// 指向某个 game root 下的单个实例(`versions/<id>/`)。
///
/// `Instance` 只持有定位信息(实例 id 与所属 game root),不缓存任何配置内容 ——
/// 配置每次按需从磁盘读取,避免内存与磁盘状态不一致。
#[derive(Debug, Clone)]
pub struct Instance {
    id: String,
    root: PathBuf,
}

impl Instance {
    /// 在 `root`(game root)下绑定 id 为 `id` 的实例。不做存在性检查 ——
    /// 调用方若需要可先用 [`Instance::dir`] / [`GamePaths::version_json`] 判断。
    pub fn new(id: impl Into<String>, root: impl Into<PathBuf>) -> Self {
        Self { id: id.into(), root: root.into() }
    }

    /// 该实例所属 game root 的目录布局视图。
    pub fn paths(&self) -> GamePaths {
        GamePaths::new(self.root.clone())
    }

    /// 实例(= 版本)id。
    pub fn version_id(&self) -> &str {
        &self.id
    }

    /// 实例目录:`<root>/versions/<id>/`。
    pub fn dir(&self) -> PathBuf {
        self.paths().version_dir(&self.id)
    }

    /// `instance.json` 的完整路径。
    pub fn config_path(&self) -> PathBuf {
        self.dir().join(INSTANCE_CONFIG_FILE)
    }

    /// 该实例的游戏运行目录(`--gameDir`)。当前模型下等于实例目录,
    /// saves/mods/resourcepacks 等运行时数据都在此(版本隔离)。
    pub fn game_dir(&self) -> PathBuf {
        self.dir()
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.game_dir().join("mods")
    }

    pub fn saves_dir(&self) -> PathBuf {
        self.game_dir().join("saves")
    }

    pub fn resourcepacks_dir(&self) -> PathBuf {
        self.game_dir().join("resourcepacks")
    }

    pub fn shaderpacks_dir(&self) -> PathBuf {
        self.game_dir().join("shaderpacks")
    }

    pub fn datapacks_dir(&self) -> PathBuf {
        self.game_dir().join("datapacks")
    }

    pub fn screenshots_dir(&self) -> PathBuf {
        self.game_dir().join("screenshots")
    }

    /// 读取该实例的配置;文件不存在时返回 [`InstanceConfig::default`]。
    pub fn load_config(&self) -> Result<InstanceConfig> {
        InstanceConfig::load(&self.config_path())
    }

    /// 将配置写入该实例的 `instance.json`(自动创建目录)。
    pub fn save_config(&self, config: &InstanceConfig) -> Result<()> {
        config.save(&self.config_path())
    }
}

/// 由实例 id 与可选的 `inheritsFrom` 推断 loader 家族。
///
/// loader 版本 json 通常带有提示性 id / `inheritsFrom`(如 `fabric-loader-0.15-1.20.1`、
/// `1.20.1-forge-47.2.0`),这里做轻量子串匹配即可,无需解析整份 json。
/// 顺序上先匹配更具体的 `neoforge`(它也包含子串 `forge`)。
fn infer_loader(id: &str, inherits_from: Option<&str>) -> LoaderKind {
    let mut hay = id.to_ascii_lowercase();
    if let Some(parent) = inherits_from {
        hay.push(' ');
        hay.push_str(&parent.to_ascii_lowercase());
    }

    if hay.contains("neoforge") {
        LoaderKind::NeoForge
    } else if hay.contains("forge") {
        LoaderKind::Forge
    } else if hay.contains("fabric") {
        LoaderKind::Fabric
    } else if hay.contains("quilt") {
        LoaderKind::Quilt
    } else if hay.contains("optifine") {
        LoaderKind::OptiFine
    } else if hay.contains("liteloader") {
        LoaderKind::LiteLoader
    } else {
        LoaderKind::Vanilla
    }
}

/// 仅提取 list 视图所需的几个字段,避免对每个实例都做完整 [`crate::version::VersionJson`]
/// 反序列化(完整结构包含 libraries/arguments 等,代价高且与列表无关)。
#[derive(serde::Deserialize)]
struct VersionHead {
    id: Option<String>,
    #[serde(rename = "inheritsFrom")]
    inherits_from: Option<String>,
}

/// 取目录的最后修改时间(epoch millis),失败返回 0。
fn dir_mtime_millis(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 从某实例的 `instance.json` 读 `last_played`。这里的"上次游玩"语义由 launch 层
/// 在启动时写入 config;若 config 不存在或未记录,则回退到实例目录的 mtime。
///
/// 注:[`InstanceConfig`] 当前不含 `last_played` 字段,故统一回退到目录 mtime;
/// 该 helper 独立出来便于将来在 config 增加该字段时只改一处。
fn last_played_for(dir: &Path) -> u64 {
    dir_mtime_millis(dir)
}

/// 沿 `inheritsFrom` 链解析到根(无 inheritsFrom 的原版版本),返回其 id 作为基础 mc 版本。
///
/// 普通 loader 实例只继承一层(loader → 原版);整合包实例可能多层
/// (实例 → loader → 原版),后者若只取一层会把 loader id 当成 mc 版本。带深度上限防御
/// 异常循环;父 json 读不到时停在当前已知最深的 id(它就是要找的根)。
fn resolve_base_mc_version(paths: &GamePaths, id: &str, inherits: Option<&str>) -> String {
    let mut base = id.to_string();
    let mut next = inherits.map(|s| s.to_string());
    let mut depth = 0;
    while let Some(parent) = next {
        if depth >= 16 {
            break;
        }
        base = parent.clone();
        next = std::fs::read_to_string(paths.version_json(&parent))
            .ok()
            .and_then(|raw| serde_json::from_str::<VersionHead>(&raw).ok())
            .and_then(|h| h.inherits_from);
        depth += 1;
    }
    base
}

/// 扫描一个 game root,列出其中所有实例。
///
/// 规则:遍历 `versions/` 下每个子目录,若存在 `versions/<id>/<id>.json` 则视为实例。
/// 对每个实例做轻量读取(只取 id / inheritsFrom)推断 loader 与基础 mc 版本。
/// 任何一步读取或解析失败的目录会被**跳过**(不 panic、不中断其余实例),
/// 保证一个坏目录不会让整个列表不可用。
pub fn list_instances(paths: &GamePaths) -> Vec<InstanceSummary> {
    let versions_dir = paths.versions_dir();

    let entries = match std::fs::read_dir(&versions_dir) {
        Ok(e) => e,
        // versions/ 不存在 = 空 root,返回空列表。
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<InstanceSummary> = Vec::new();

    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }

        // 目录名即版本 id。
        let dir_id = match dir.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => continue,
        };

        // 必须存在 <id>/<id>.json 才算实例。
        let json_path = paths.version_json(&dir_id);
        let raw = match std::fs::read_to_string(&json_path) {
            Ok(r) => r,
            Err(_) => continue, // 没有版本 json,跳过(不是实例,如纯 natives/临时目录)。
        };

        // 轻量解析;解析失败的损坏 json 跳过。
        let head: VersionHead = match serde_json::from_str(&raw) {
            Ok(h) => h,
            Err(_) => continue,
        };

        // 优先用 json 内的 id,回退到目录名。
        let id = head.id.filter(|s| !s.is_empty()).unwrap_or_else(|| dir_id.clone());
        let inherits = head.inherits_from.clone();
        let loader = infer_loader(&id, inherits.as_deref());

        // 基础 mc 版本:沿 inheritsFrom 链解析到根原版(整合包实例可能多层继承)。
        let mc_version = resolve_base_mc_version(paths, &id, inherits.as_deref());

        // 实例名优先取 instance.json 的 name,缺省用 id。
        let config = InstanceConfig::load(&dir.join(INSTANCE_CONFIG_FILE)).unwrap_or_default();
        let name = config.name.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| id.clone());

        out.push(InstanceSummary {
            id,
            name,
            mc_version,
            loader,
            // 非原版才暴露 loader_version:用整体 id 作为可读标识(精确解析留给 meta 层)。
            loader_version: match loader {
                LoaderKind::Vanilla => None,
                _ => Some(dir_id.clone()),
            },
            icon: None, // 图标当前未在 config 建模,留空(后续可由 icon.png 探测填充)。
            last_played: last_played_for(&dir),
            running: false, // 运行态由上层进程管理器维护,列表层不感知。
        });
    }

    // 稳定排序:按 id 字典序,保证列表展示顺序确定。
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    /// 在临时目录里搭一个假的 game root,测试结束自动清理。
    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!("mc-core-instance-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        /// 写入 versions/<id>/<id>.json(可选附带 inheritsFrom)。
        fn add_version(&self, id: &str, inherits: Option<&str>) {
            let paths = GamePaths::new(self.path.clone());
            let dir = paths.version_dir(id);
            fs::create_dir_all(&dir).unwrap();
            let json = match inherits {
                Some(p) => format!(r#"{{"id":"{id}","inheritsFrom":"{p}"}}"#),
                None => format!(r#"{{"id":"{id}"}}"#),
            };
            fs::write(paths.version_json(id), json).unwrap();
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn missing_versions_dir_is_empty() {
        let root = TempRoot::new("empty");
        let paths = GamePaths::new(root.path.clone());
        assert!(list_instances(&paths).is_empty());
    }

    #[test]
    fn lists_vanilla_and_loader_instances() {
        let root = TempRoot::new("mixed");
        root.add_version("1.20.1", None);
        root.add_version("fabric-loader-0.15.7-1.20.1", Some("1.20.1"));
        root.add_version("1.20.1-forge-47.2.0", Some("1.20.1"));

        // 一个没有版本 json 的目录(如 natives 残留),应被跳过。
        fs::create_dir_all(root.path.join("versions").join("junk")).unwrap();
        // 一个 json 损坏的目录,应被跳过而不 panic。
        let bad_dir = root.path.join("versions").join("broken");
        fs::create_dir_all(&bad_dir).unwrap();
        fs::write(bad_dir.join("broken.json"), "{ not valid json ").unwrap();

        let paths = GamePaths::new(root.path.clone());
        let list = list_instances(&paths);

        assert_eq!(list.len(), 3, "should list exactly the 3 valid instances");

        let by_id = |id: &str| list.iter().find(|s| s.id == id).cloned().unwrap();

        let vanilla = by_id("1.20.1");
        assert_eq!(vanilla.loader, LoaderKind::Vanilla);
        assert_eq!(vanilla.mc_version, "1.20.1");
        assert!(vanilla.loader_version.is_none());

        let fabric = by_id("fabric-loader-0.15.7-1.20.1");
        assert_eq!(fabric.loader, LoaderKind::Fabric);
        assert_eq!(fabric.mc_version, "1.20.1");
        assert!(fabric.loader_version.is_some());

        let forge = by_id("1.20.1-forge-47.2.0");
        assert_eq!(forge.loader, LoaderKind::Forge);
        assert_eq!(forge.mc_version, "1.20.1");
    }

    #[test]
    fn instance_name_comes_from_config() {
        let root = TempRoot::new("named");
        root.add_version("1.20.1", None);

        let inst = Instance::new("1.20.1", root.path.clone());
        let mut cfg = InstanceConfig::default();
        cfg.name = Some("Survival World".to_string());
        inst.save_config(&cfg).unwrap();

        let paths = GamePaths::new(root.path.clone());
        let list = list_instances(&paths);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Survival World");
        assert_eq!(list[0].id, "1.20.1");
    }

    #[test]
    fn instance_config_roundtrip_via_helper() {
        let root = TempRoot::new("cfg");
        let inst = Instance::new("test-id", root.path.clone());

        // 未写盘时返回默认。
        let def = inst.load_config().unwrap();
        assert_eq!(def, InstanceConfig::default());
        assert_eq!(inst.version_id(), "test-id");
        assert!(inst.config_path().ends_with("versions/test-id/instance.json"));

        let mut cfg = InstanceConfig::default();
        cfg.memory_mb = 6144;
        inst.save_config(&cfg).unwrap();
        assert_eq!(inst.load_config().unwrap().memory_mb, 6144);
    }

    #[test]
    fn infer_loader_precedence() {
        assert_eq!(infer_loader("neoforge-1.21", None), LoaderKind::NeoForge);
        // neoforge 含 "forge" 子串,但应判定为 NeoForge。
        assert_eq!(infer_loader("1.21-neoforge-21.0.0", Some("1.21")), LoaderKind::NeoForge);
        assert_eq!(infer_loader("1.20.1-forge-47.2.0", Some("1.20.1")), LoaderKind::Forge);
        assert_eq!(infer_loader("fabric-loader-0.15", Some("1.20.1")), LoaderKind::Fabric);
        assert_eq!(infer_loader("quilt-loader-0.20", Some("1.20.1")), LoaderKind::Quilt);
        assert_eq!(infer_loader("1.20.1-OptiFine_HD_U_I6", None), LoaderKind::OptiFine);
        assert_eq!(infer_loader("1.5.2-LiteLoader1.5.2", None), LoaderKind::LiteLoader);
        assert_eq!(infer_loader("1.20.1", None), LoaderKind::Vanilla);
    }
}
