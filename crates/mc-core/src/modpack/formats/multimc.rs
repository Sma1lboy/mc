//! MultiMC / Prism 原生格式(目录即包)。
//!
//! 不是单文件,是实例**目录**(导出 = 该目录打 zip)。两文件驱动:
//! - `mmc-pack.json`:组件图([`MmcPack`] / [`MmcComponent`] / [`MmcRequire`]),`formatVersion==1`,顺序即合并序。
//! - `instance.cfg`:**无 section 的 INI**(Qt QSettings),由 [`parse_instance_cfg`] 解析成
//!   [`InstanceCfg`](typed view + raw map,无损往返)。
//!
//! 游戏目录是 `minecraft/`(回退 `.minecraft/`),内容是预装好的 loose 文件,无远程 `files[]`。
//!
//! 易错点(对照 `docs/modules/modpack-formats.md` §4):
//! - **bug-for-bug**:Prism 写 `cachedVolatile` 但读 `volatile` → 输入接受两者(`alias`),输出写 `cachedVolatile`。
//! - `MmcRequire` 的 `equals_version` JSON 键是 `"equals"`。
//! - `instance.cfg` 的 `Override*` 闸:只在闸=true 时对应键才生效,否则继承全局 → 映射统一模型时用 `Option`。
//! - 加载器 uid 表见 [`MmcLoaderKind`]:`org.lwjgl*` / `net.minecraft` **不是** loader。
//! - **导入的 JavaPath / 启动命令不可自动信任**(会执行任意二进制),由上层显式确认。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ===========================================================================
// mmc-pack.json
// ===========================================================================

fn is_false(b: &bool) -> bool {
    !*b
}

/// `mmc-pack.json` 顶层:组件图。`formatVersion` 必须 1;`components` 顺序即合并序。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmcPack {
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    pub components: Vec<MmcComponent>,
}

/// 组件图里的一个组件(原版 / loader / lwjgl / jarmod …)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmcComponent {
    /// 组件 uid,见 [`MmcLoaderKind::from_uid`]。
    pub uid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 仅作为依赖被引入(非用户直接选)。
    #[serde(default, skip_serializing_if = "is_false", rename = "dependencyOnly")]
    pub dependency_only: bool,
    /// `net.minecraft` / 所选 loader → 不可删。
    #[serde(default, skip_serializing_if = "is_false")]
    pub important: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cachedVersion")]
    pub cached_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cachedName")]
    pub cached_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "cachedRequires")]
    pub cached_requires: Vec<MmcRequire>,
    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "cachedConflicts")]
    pub cached_conflicts: Vec<MmcRequire>,
    /// bug-for-bug:Prism 写 `cachedVolatile` 但读 `volatile` → 输入接受两者,输出写 `cachedVolatile`。
    #[serde(
        default,
        skip_serializing_if = "is_false",
        rename = "cachedVolatile",
        alias = "volatile"
    )]
    pub cached_volatile: bool,
}

/// 组件的依赖 / 冲突约束。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MmcRequire {
    pub uid: String,
    /// JSON 键是 `"equals"`(精确版本约束)。
    #[serde(default, rename = "equals", skip_serializing_if = "Option::is_none")]
    pub equals_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggests: Option<String>,
}

/// 已知加载器 uid 表(`KNOWN_MODLOADERS`)。
///
/// `org.lwjgl*` / `net.minecraft` **不是** loader,映射为 `None`。语义提示:
/// Quilt 蕴含 Fabric 支持;1.20.1 上 NeoForge 蕴含 Forge(由上层 loader 安装逻辑处理)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MmcLoaderKind {
    NeoForge,
    Forge,
    Fabric,
    Quilt,
    LiteLoader,
}

impl MmcLoaderKind {
    /// 由组件 uid 判定 loader 家族。非 loader uid(原版 / lwjgl / jarmod …)返回 `None`。
    pub fn from_uid(uid: &str) -> Option<MmcLoaderKind> {
        match uid {
            "net.neoforged" => Some(MmcLoaderKind::NeoForge),
            "net.minecraftforge" => Some(MmcLoaderKind::Forge),
            "net.fabricmc.fabric-loader" => Some(MmcLoaderKind::Fabric),
            "org.quiltmc.quilt-loader" => Some(MmcLoaderKind::Quilt),
            "com.mumfrey.liteloader" => Some(MmcLoaderKind::LiteLoader),
            _ => None,
        }
    }
}

impl MmcPack {
    /// 找到原版组件(`uid == "net.minecraft"`)的版本 = MC 版本。
    pub fn minecraft_version(&self) -> Option<&str> {
        self.components
            .iter()
            .find(|c| c.uid == "net.minecraft")
            .and_then(|c| c.version.as_deref())
    }

    /// 找到生效的 loader 组件(第一个匹配 [`MmcLoaderKind`] 的)及其家族。
    pub fn loader(&self) -> Option<(MmcLoaderKind, &MmcComponent)> {
        self.components
            .iter()
            .find_map(|c| MmcLoaderKind::from_uid(&c.uid).map(|k| (k, c)))
    }
}

// ===========================================================================
// instance.cfg(无 section INI,Qt QSettings)
// ===========================================================================

/// `instance.cfg` 的类型化视图 + 原始全键 map(无损往返)。
///
/// `Override*` 闸用 `Option`:闸=false 时对应字段为 `None`(继承全局),只有闸=true
/// 且键存在时才填值。`raw` 保留所有键(含未建模的)以便无损回写。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstanceCfg {
    /// 实例显示名(`name`)。
    pub name: Option<String>,
    /// `InstanceType`(通常 `OneSix`)。
    pub instance_type: Option<String>,
    /// 图标键(`iconKey`)。
    pub icon_key: Option<String>,
    /// 备注(`notes`)。
    pub notes: Option<String>,
    /// 内存:仅当 `OverrideMemory=true` 时填(否则继承全局)。
    pub min_mem_alloc: Option<u32>,
    pub max_mem_alloc: Option<u32>,
    /// JVM 参数:仅当 `OverrideJavaArgs=true` 时填。
    pub jvm_args: Option<String>,
    /// Java 路径:仅当 `OverrideJavaLocation=true` 时填。**不可自动信任**(见模块文档)。
    pub java_path: Option<String>,
    /// 跨格式溯源(`ManagedPackType`):这实例来自 modrinth / flame / …。
    pub managed_pack: Option<String>,
    pub managed_pack_type: Option<String>,
    pub managed_pack_id: Option<String>,
    pub managed_pack_version_id: Option<String>,
    /// 全部原始键(无损往返)。
    pub raw: BTreeMap<String, String>,
}

/// 把 `instance.cfg` 文本(无 section INI,Qt QSettings 风格)解析成 [`InstanceCfg`]。
///
/// 解析规则:
/// - 按行;`#` / `;` 起头的整行是注释;空行忽略。
/// - `[section]` 头被忽略(QSettings 实例文件本无 section,但容忍存在)。
/// - 首个 `=` 切分键 / 值,两侧 trim。重复键后者覆盖。
/// - `Override*` 闸为 `true`(忽略大小写)才让对应的受闸键进入 typed 字段。
///
/// 不做布尔/数字校验失败即中断:数字解析失败的内存键当作未设(`None`),但仍保留在 `raw`。
pub fn parse_instance_cfg(text: &str) -> InstanceCfg {
    let mut raw: BTreeMap<String, String> = BTreeMap::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }
        // 容忍并跳过 section 头。
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        raw.insert(k.trim().to_string(), v.trim().to_string());
    }

    let bool_gate = |key: &str| -> bool {
        raw.get(key)
            .map(|v| v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    };
    let get = |key: &str| -> Option<String> { raw.get(key).cloned().filter(|s| !s.is_empty()) };
    let get_u32 = |key: &str| -> Option<u32> { raw.get(key).and_then(|v| v.trim().parse().ok()) };

    let override_memory = bool_gate("OverrideMemory");
    let override_java_args = bool_gate("OverrideJavaArgs");
    let override_java_location = bool_gate("OverrideJavaLocation");

    InstanceCfg {
        name: get("name"),
        instance_type: get("InstanceType"),
        icon_key: get("iconKey"),
        notes: get("notes"),
        min_mem_alloc: if override_memory { get_u32("MinMemAlloc") } else { None },
        max_mem_alloc: if override_memory { get_u32("MaxMemAlloc") } else { None },
        jvm_args: if override_java_args { get("JvmArgs") } else { None },
        java_path: if override_java_location { get("JavaPath") } else { None },
        managed_pack: get("ManagedPack"),
        managed_pack_type: get("ManagedPackType"),
        managed_pack_id: get("ManagedPackID"),
        managed_pack_version_id: get("ManagedPackVersionID"),
        raw,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mmc_pack_with_loader_and_mc_version() {
        let sample = r#"{
            "formatVersion": 1,
            "components": [
                { "uid": "net.minecraft", "version": "1.20.1", "important": true },
                { "uid": "org.lwjgl3", "version": "3.3.1", "dependencyOnly": true },
                {
                    "uid": "net.fabricmc.fabric-loader",
                    "version": "0.15.7",
                    "important": true,
                    "cachedName": "Fabric Loader",
                    "cachedRequires": [ { "uid": "net.minecraft", "equals": "1.20.1" } ]
                }
            ]
        }"#;
        let pack: MmcPack = serde_json::from_str(sample).unwrap();
        assert_eq!(pack.format_version, 1);
        assert_eq!(pack.minecraft_version(), Some("1.20.1"));

        let (kind, comp) = pack.loader().unwrap();
        assert_eq!(kind, MmcLoaderKind::Fabric);
        assert_eq!(comp.version.as_deref(), Some("0.15.7"));
        assert_eq!(comp.cached_requires[0].equals_version.as_deref(), Some("1.20.1"));

        // lwjgl / minecraft 不是 loader。
        assert_eq!(MmcLoaderKind::from_uid("org.lwjgl3"), None);
        assert_eq!(MmcLoaderKind::from_uid("net.minecraft"), None);
    }

    #[test]
    fn volatile_alias_accepts_both_and_serializes_cached() {
        // 读 "volatile" 也接受。
        let read_volatile: MmcComponent =
            serde_json::from_str(r#"{ "uid": "x", "volatile": true }"#).unwrap();
        assert!(read_volatile.cached_volatile);

        // 读 "cachedVolatile" 也接受。
        let read_cached: MmcComponent =
            serde_json::from_str(r#"{ "uid": "x", "cachedVolatile": true }"#).unwrap();
        assert!(read_cached.cached_volatile);

        // 写出用 "cachedVolatile"。
        let json = serde_json::to_string(&read_cached).unwrap();
        assert!(json.contains("cachedVolatile"));
        assert!(!json.contains("\"volatile\""));
    }

    #[test]
    fn neoforge_and_forge_uid_table() {
        assert_eq!(MmcLoaderKind::from_uid("net.neoforged"), Some(MmcLoaderKind::NeoForge));
        assert_eq!(MmcLoaderKind::from_uid("net.minecraftforge"), Some(MmcLoaderKind::Forge));
        assert_eq!(MmcLoaderKind::from_uid("org.quiltmc.quilt-loader"), Some(MmcLoaderKind::Quilt));
        assert_eq!(MmcLoaderKind::from_uid("com.mumfrey.liteloader"), Some(MmcLoaderKind::LiteLoader));
        assert_eq!(MmcLoaderKind::from_uid("custom.jarmod.abc"), None);
    }

    #[test]
    fn instance_cfg_honors_override_gates() {
        let text = "\
name=My Pack
InstanceType=OneSix
iconKey=flame
OverrideMemory=true
MinMemAlloc=2048
MaxMemAlloc=8192
OverrideJavaArgs=false
JvmArgs=-XX:+UseG1GC
OverrideJavaLocation=true
JavaPath=/opt/java/bin/java
ManagedPackType=modrinth
ManagedPackID=AABBCCDD
# a comment line
[General]
notes=hello world
";
        let cfg = parse_instance_cfg(text);
        assert_eq!(cfg.name.as_deref(), Some("My Pack"));
        assert_eq!(cfg.instance_type.as_deref(), Some("OneSix"));
        assert_eq!(cfg.icon_key.as_deref(), Some("flame"));
        assert_eq!(cfg.notes.as_deref(), Some("hello world"));

        // OverrideMemory=true → 内存键生效。
        assert_eq!(cfg.min_mem_alloc, Some(2048));
        assert_eq!(cfg.max_mem_alloc, Some(8192));

        // OverrideJavaArgs=false → JvmArgs 不进 typed(继承全局),但仍在 raw。
        assert_eq!(cfg.jvm_args, None);
        assert_eq!(cfg.raw.get("JvmArgs").map(String::as_str), Some("-XX:+UseG1GC"));

        // OverrideJavaLocation=true → JavaPath 生效(但上层须显式确认才信任)。
        assert_eq!(cfg.java_path.as_deref(), Some("/opt/java/bin/java"));

        // 溯源。
        assert_eq!(cfg.managed_pack_type.as_deref(), Some("modrinth"));
        assert_eq!(cfg.managed_pack_id.as_deref(), Some("AABBCCDD"));

        // raw 保留全键(含 section 内的 notes 与各 Override 闸)。
        assert_eq!(cfg.raw.get("OverrideMemory").map(String::as_str), Some("true"));
        assert_eq!(cfg.raw.get("name").map(String::as_str), Some("My Pack"));
    }

    #[test]
    fn instance_cfg_memory_gate_off_means_none() {
        let text = "name=X\nMinMemAlloc=2048\nMaxMemAlloc=4096\n";
        let cfg = parse_instance_cfg(text);
        // 没有 OverrideMemory=true → 内存继承全局(None),但 raw 仍有值。
        assert_eq!(cfg.min_mem_alloc, None);
        assert_eq!(cfg.raw.get("MinMemAlloc").map(String::as_str), Some("2048"));
    }
}
