//! 精简版组件版本模型(MultiMC/Prism `mmc-pack.json`)。
//!
//! 见 `docs/modules/instance-and-components.md` 与 `docs/modules/modpack-formats.md` §4。
//!
//! mc-core 现状是**扁平**的:每个 loader json 用 `inheritsFrom` 指向 vanilla,合并序由
//! 父指针隐式决定(见 [`super::load_chain`])。本模块引入**有序组件列表**——版本定义
//! 变成 `[{uid, version}, ...]`,**顺序即合并优先级**(后盖前)。这样才能把
//! `net.fabricmc.intermediary` 夹在 `net.minecraft` 与 loader 之间,并把"加载器身份"
//! 从"按 id 子串猜"(`instance/mod.rs::infer_loader`)升级为"列表里哪个已知 loader uid"。
//!
//! 本文件只含**纯数据 + 同步逻辑**:serde 结构、`KNOWN_LOADERS` 表、`mmc-pack.json`
//! 读写、以及一个有原则的依赖解析器(intermediary == mc 作规则,**不**硬编码 lwjgl 表)。
//! IO 像 `load_chain` 那样由调用方注入,故全部可单测、无网络。

use std::path::Path;

use serde::{Deserialize, Serialize};

use mc_types::LoaderKind;

use crate::error::{CoreError, Result};

/// `mmc-pack.json` 在实例目录下的固定文件名(MultiMC/Prism 兼容)。
pub const PACK_FILE: &str = "mmc-pack.json";

/// 原版组件 uid(锚组件,`important`,不可删)。
pub const UID_MINECRAFT: &str = "net.minecraft";
/// Fabric 的中间映射组件 uid(`dependency_only`,版本 == mc 版本)。
pub const UID_FABRIC_INTERMEDIARY: &str = "net.fabricmc.intermediary";
/// Quilt 的中间映射组件 uid(`dependency_only`,版本 == mc 版本)。
pub const UID_QUILT_HASHED: &str = "org.quiltmc.hashed";

/// `serde(skip_serializing_if)` 用:布尔为 false 时不写出该键。
fn is_false(b: &bool) -> bool {
    !*b
}

/// 跨组件依赖边(一条 require 或 conflict)。按 `uid` 去重。
///
/// JSON 形如 `{"uid":"net.minecraft","equals":"1.20.1","suggests":"1.20.1"}`:
/// - `equals`(Rust 字段 `equals_version`)— **硬约束**:被依赖组件的版本必须等于它。
/// - `suggests` — 软约束:解析器在无 `equals` 时取各依赖者建议中的最大值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Require {
    pub uid: String,
    /// JSON 键是 `"equals"`(不是 `equals_version`),与 Prism 一致。
    #[serde(default, rename = "equals", skip_serializing_if = "Option::is_none")]
    pub equals_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggests: Option<String>,
}

impl Require {
    /// 仅有 uid 的 require(版本未约束)。
    pub fn new(uid: impl Into<String>) -> Self {
        Self { uid: uid.into(), equals_version: None, suggests: None }
    }

    /// `equals` 硬约束的 require(如 intermediary == mc 版本)。
    pub fn equals(uid: impl Into<String>, version: impl Into<String>) -> Self {
        Self { uid: uid.into(), equals_version: Some(version.into()), suggests: None }
    }
}

/// 组件图里的一个组件(一层版本定义)。
///
/// 字段级 serde 规则严格对照 `docs/modules/modpack-formats.md` §4:布尔默认 false 且
/// false 时不写出;`Option`/`Vec` 缺省/空时不写出;`cached*` 是离线快照(取 meta 版本
/// 文件时刷新),无网络也能解析依赖。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    /// 组件唯一标识:`net.minecraft` / `net.fabricmc.fabric-loader` /
    /// `net.minecraftforge` / `net.neoforged` / `org.quiltmc.quilt-loader` /
    /// `net.fabricmc.intermediary` / `org.lwjgl3` / `custom.jarmod.<uuid>` …
    pub uid: String,

    /// 该组件选定的版本;`None` 表示"待解析/浮动"(尚未定版)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    /// 自动注入的依赖组件(如 intermediary / lwjgl);无人依赖且 `cached_volatile`
    /// 时会被解析器移除。
    #[serde(default, skip_serializing_if = "is_false", rename = "dependencyOnly")]
    pub dependency_only: bool,

    /// `net.minecraft` 与所选 loader → `important`,不可删。
    #[serde(default, skip_serializing_if = "is_false")]
    pub important: bool,

    /// 该层被临时禁用(参与列表但不参与合并)。
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cachedVersion")]
    pub cached_version: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cachedName")]
    pub cached_name: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "cachedRequires")]
    pub cached_requires: Vec<Require>,

    #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "cachedConflicts")]
    pub cached_conflicts: Vec<Require>,

    /// `volatile` 组件:不再被需要时自动移除(典型为 intermediary/lwjgl)。
    ///
    /// bug-for-bug 兼容 Prism:它**写** `cachedVolatile` 但**读** `volatile`,故输入
    /// 两个键都接受(`alias`),输出统一写 `cachedVolatile`。
    #[serde(
        default,
        skip_serializing_if = "is_false",
        rename = "cachedVolatile",
        alias = "volatile"
    )]
    pub cached_volatile: bool,
}

impl Component {
    /// 普通组件(仅 uid + 版本)。
    pub fn new(uid: impl Into<String>, version: Option<String>) -> Self {
        Self {
            uid: uid.into(),
            version,
            dependency_only: false,
            important: false,
            disabled: false,
            cached_version: None,
            cached_name: None,
            cached_requires: Vec::new(),
            cached_conflicts: Vec::new(),
            cached_volatile: false,
        }
    }

    /// 锚/loader 这类不可删组件(`important`)。
    pub fn important(uid: impl Into<String>, version: Option<String>) -> Self {
        Self { important: true, ..Self::new(uid, version) }
    }

    /// 自动注入的依赖组件(`dependency_only` + `volatile`),如 intermediary。
    pub fn dependency(uid: impl Into<String>, version: Option<String>) -> Self {
        Self {
            dependency_only: true,
            cached_volatile: true,
            ..Self::new(uid, version)
        }
    }

    /// 该组件是否参与合并(未禁用)。
    pub fn is_active(&self) -> bool {
        !self.disabled
    }
}

/// 已知加载器在组件模型里的身份(uid → 家族 + 互斥 uid 集)。
///
/// 把"加载器识别"从启发式子串匹配升级为 uid 注册表 + 冲突矩阵。
pub struct KnownLoader {
    /// 该 loader 的组件 uid。
    pub uid: &'static str,
    /// 家族,映射到现有的 [`LoaderKind`]。
    pub kind: LoaderKind,
    /// 与之互斥的其它 loader uid(共存即冲突)。
    ///
    /// 注:同列表里两两都是 loader 即互斥;此处只列**额外**的语义关系并无必要——
    /// 解析器把"任意两个不同 loader uid 共存"判为冲突(见 [`PackProfile::loader_conflict`]),
    /// 故 `conflicts` 留给文档化的特殊蕴含关系(如 Quilt 蕴含 Fabric、1.20.1 NeoForge
    /// 蕴含 Forge),当前不参与互斥判定。
    pub conflicts: &'static [&'static str],
}

/// 已知加载器表(`KNOWN_LOADERS`)。`org.lwjgl*` / `net.minecraft` / intermediary /
/// hashed **不是** loader,故不在表内。
///
/// 与 `docs/modules/modpack-formats.md` §4 的 `KNOWN_MODLOADERS` 一致。
pub const KNOWN_LOADERS: &[KnownLoader] = &[
    KnownLoader {
        uid: "net.neoforged",
        kind: LoaderKind::NeoForge,
        // 1.20.1 上 NeoForge 蕴含 Forge(文档化关系,非互斥判定)。
        conflicts: &["net.minecraftforge"],
    },
    KnownLoader {
        uid: "net.minecraftforge",
        kind: LoaderKind::Forge,
        conflicts: &["net.neoforged"],
    },
    KnownLoader {
        uid: "net.fabricmc.fabric-loader",
        kind: LoaderKind::Fabric,
        conflicts: &["org.quiltmc.quilt-loader"],
    },
    KnownLoader {
        uid: "org.quiltmc.quilt-loader",
        kind: LoaderKind::Quilt,
        // Quilt 蕴含 Fabric 支持(文档化关系,非互斥判定)。
        conflicts: &["net.fabricmc.fabric-loader"],
    },
    KnownLoader {
        uid: "com.mumfrey.liteloader",
        kind: LoaderKind::LiteLoader,
        conflicts: &[],
    },
];

/// 查某个 uid 对应的已知 loader(若它是 loader)。
pub fn known_loader(uid: &str) -> Option<&'static KnownLoader> {
    KNOWN_LOADERS.iter().find(|l| l.uid == uid)
}

/// 把 [`LoaderKind`] 映射回它在组件模型里的 uid(用于"从零/导入"建组件)。
///
/// `Vanilla` 无 loader uid → `None`;`OptiFine` 当前不在组件 loader 表内(它在
/// mc-core 里作为 vanilla 之上的特殊层处理)→ `None`。
pub fn loader_uid(kind: LoaderKind) -> Option<&'static str> {
    KNOWN_LOADERS.iter().find(|l| l.kind == kind).map(|l| l.uid)
}

/// 实例目录里的版本定义:有序组件列表。落地为 `mmc-pack.json`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackProfile {
    /// 格式版本,规范为 1。
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    /// **有序**组件列表:顺序即合并优先级(后盖前)。
    #[serde(default)]
    pub components: Vec<Component>,
}

impl Default for PackProfile {
    fn default() -> Self {
        Self { format_version: 1, components: Vec::new() }
    }
}

impl PackProfile {
    /// 空 pack(`formatVersion=1`,无组件)。等价 `default()`,语义化命名。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置(或更新)某 uid 组件的版本与 `important` 标记。
    ///
    /// - 已存在该 uid:就地更新 `version`,并将 `important` 取**或**(置位不复位,
    ///   避免覆盖此前显式 important);其余字段保留。
    /// - 不存在:**追加**到列表末尾(对齐 Prism `setComponentVersion` 的"追加锚"语义,
    ///   合并序由列表顺序决定,intermediary 等依赖项由解析器插到正确位置)。
    pub fn set_component(&mut self, uid: &str, version: impl Into<String>, important: bool) {
        let version = version.into();
        if let Some(c) = self.components.iter_mut().find(|c| c.uid == uid) {
            c.version = Some(version);
            c.important = c.important || important;
        } else {
            let mut c = Component::new(uid, Some(version));
            c.important = important;
            self.components.push(c);
        }
    }

    /// 取某 uid 组件(只读)。
    pub fn get_component(&self, uid: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.uid == uid)
    }

    /// 取某 uid 组件(可变)。
    pub fn get_component_mut(&mut self, uid: &str) -> Option<&mut Component> {
        self.components.iter_mut().find(|c| c.uid == uid)
    }

    /// 是否存在该 uid 组件。
    pub fn has_component(&self, uid: &str) -> bool {
        self.components.iter().any(|c| c.uid == uid)
    }

    /// 锚组件 `net.minecraft` 的版本(若已定版)。
    pub fn minecraft_version(&self) -> Option<&str> {
        self.get_component(UID_MINECRAFT).and_then(|c| c.version.as_deref())
    }

    /// 检测列表里出现的(第一个,active 且非依赖项)已知 loader uid + 家族。
    ///
    /// 用它替代 `infer_loader` 的子串猜测:loader 身份直接来自组件 uid。无 loader 组件
    /// 则视为 [`LoaderKind::Vanilla`]。
    pub fn detect_loader(&self) -> LoaderKind {
        self.components
            .iter()
            .filter(|c| c.is_active() && !c.dependency_only)
            .find_map(|c| known_loader(&c.uid).map(|l| l.kind))
            .unwrap_or(LoaderKind::Vanilla)
    }

    /// 检测到的 loader 组件(uid+version),无则 `None`。
    pub fn detect_loader_component(&self) -> Option<&Component> {
        self.components
            .iter()
            .filter(|c| c.is_active() && !c.dependency_only)
            .find(|c| known_loader(&c.uid).is_some())
    }

    /// 列表里是否存在**两个及以上**互斥 loader(active)——双 loader 共存冲突。
    ///
    /// 把"任意两个不同 loader uid 共存"判为冲突(精简且充分:同一实例不应有两个核心)。
    pub fn loader_conflict(&self) -> Option<(String, String)> {
        let loaders: Vec<&str> = self
            .components
            .iter()
            .filter(|c| c.is_active() && !c.dependency_only)
            .filter(|c| known_loader(&c.uid).is_some())
            .map(|c| c.uid.as_str())
            .collect();
        if loaders.len() >= 2 {
            Some((loaders[0].to_string(), loaders[1].to_string()))
        } else {
            None
        }
    }

    /// 从实例目录读取 `mmc-pack.json`。
    ///
    /// 文件不存在视为"尚无组件模型",返回 `Ok(None)`(调用方可回退到旧的扁平
    /// `inheritsFrom` 模型);仅当文件存在但读取/解析失败时返回错误。
    pub fn load(instance_dir: &Path) -> Result<Option<Self>> {
        let path = instance_dir.join(PACK_FILE);
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                let pack: PackProfile = serde_json::from_str(&raw)
                    .map_err(|e| CoreError::Parse { what: PACK_FILE.to_string(), source: e })?;
                Ok(Some(pack))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(CoreError::io(&path, e)),
        }
    }

    /// 把组件模型写入实例目录的 `mmc-pack.json`(美化输出,自动建目录,原子落盘)。
    pub fn save(&self, instance_dir: &Path) -> Result<()> {
        crate::paths::ensure_dir(instance_dir)?;
        let path = instance_dir.join(PACK_FILE);
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| CoreError::Parse { what: PACK_FILE.to_string(), source: e })?;
        crate::fs::write_atomic(&path, json.as_bytes())
    }

    /// 由 `(mc_version, loader, loader_version)` 直接合成一个 2 组件 pack
    /// (vanilla + loader),供"从零/导入"装核心走同一条路径。
    ///
    /// - 始终追加 `net.minecraft`(`important`)= `mc_version`。
    /// - 若 `loader != Vanilla` 且该家族有组件 uid([`loader_uid`]),再追加一个
    ///   loader 组件(`important`)= `loader_version`。
    /// - 之后调用方应跑 [`PackProfile::resolve`] 注入 intermediary/hashed 等依赖项。
    ///
    /// `OptiFine` 等无组件 uid 的家族:只产出 vanilla 锚组件(loader 由其它途径叠加)。
    pub fn from_loader(
        mc_version: &str,
        loader: LoaderKind,
        loader_version: Option<&str>,
    ) -> Self {
        let mut pack = PackProfile::new();
        pack.components.push(Component::important(UID_MINECRAFT, Some(mc_version.to_string())));
        if loader != LoaderKind::Vanilla {
            if let Some(uid) = loader_uid(loader) {
                pack.components.push(Component::important(uid, loader_version.map(str::to_string)));
            }
        }
        pack
    }

    /// 精简版依赖解析器(纯数据,无 IO):把声明 loader 所需的 `dependency_only`
    /// 组件填齐,并移除不再被需要的 volatile 依赖项。
    ///
    /// 解析规则(**有原则、非硬编码 lwjgl 表**):
    /// 1. 锚定 mc 版本 = `net.minecraft` 的 `version`(无则无可解析,直接返回)。
    /// 2. **注入缺失依赖**:每个 active loader 组件按 [`loader_requires`] 推出它需要的
    ///    `dependency_only` 组件(目前 Fabric→intermediary==mc、Quilt→hashed==mc),
    ///    若列表缺失则**插到该 loader 组件之前**(保证合并序:vanilla → dep → loader)。
    /// 3. **就地改版本**:已存在但版本不符 `equals` 的依赖项,改成 `equals` 要求的版本。
    /// 4. **平凡移除**:`dependency_only` + `volatile` 且当前无任何 active 组件再需要它,
    ///    移除。
    ///
    /// 循环到稳定(无更多增删改),最多迭代 [`MAX_RESOLVE_ITERS`] 次防发散。返回是否
    /// 发生过改动。
    pub fn resolve(&mut self) -> bool {
        let mc_version = match self.minecraft_version() {
            Some(v) => v.to_string(),
            None => return false,
        };

        let mut changed_any = false;
        for _ in 0..MAX_RESOLVE_ITERS {
            let mut changed = false;

            // ---- 收集本轮所有 active 组件提出的 require(uid → equals 版本) ----
            // 来源:已知 loader 的内建 require(loader_requires) + 组件自带 cached_requires。
            let mut required: Vec<(String, Require)> = Vec::new();
            for c in self.components.iter().filter(|c| c.is_active()) {
                if let Some(reqs) = loader_requires(&c.uid, &mc_version) {
                    for r in reqs {
                        required.push((c.uid.clone(), r));
                    }
                }
                for r in &c.cached_requires {
                    required.push((c.uid.clone(), r.clone()));
                }
            }

            // ---- 注入缺失的 require 目标(作 dependency_only),插到首个依赖者之前 ----
            for (depender_uid, req) in &required {
                if self.has_component(&req.uid) {
                    continue;
                }
                // 找首个依赖该 require 的组件位置,插到它之前(保证 dep 先于使用者合并)。
                let pos = self
                    .components
                    .iter()
                    .position(|c| &c.uid == depender_uid)
                    .unwrap_or(self.components.len());
                let mut comp = Component::dependency(req.uid.clone(), req.equals_version.clone());
                if comp.version.is_none() {
                    comp.version = req.suggests.clone();
                }
                self.components.insert(pos, comp);
                changed = true;
            }

            // ---- 就地修正版本:有 equals 硬约束但版本不符的依赖项,改成 equals ----
            for (_, req) in &required {
                if let Some(want) = &req.equals_version {
                    if let Some(c) = self.get_component_mut(&req.uid) {
                        if c.version.as_deref() != Some(want.as_str()) {
                            c.version = Some(want.clone());
                            changed = true;
                        }
                    }
                }
            }

            // ---- 平凡移除:dependency_only + volatile 且无人再需要 ----
            // 先算"仍被需要的 uid 集合"(任一 active 组件 require 它)。
            let still_needed: std::collections::HashSet<String> =
                required.iter().map(|(_, r)| r.uid.clone()).collect();
            let before = self.components.len();
            self.components.retain(|c| {
                let removable = c.dependency_only && c.cached_volatile && !still_needed.contains(&c.uid);
                !removable
            });
            if self.components.len() != before {
                changed = true;
            }

            if changed {
                changed_any = true;
            } else {
                break;
            }
        }
        changed_any
    }
}

/// 解析器最多迭代次数(注入会产生新 require,但本模型的依赖图极浅,几轮即稳定)。
const MAX_RESOLVE_ITERS: usize = 8;

/// 某个已知 loader uid 在给定 mc 版本下**内建**需要的依赖项(require 边)。
///
/// 这是"intermediary/hashed == mc 版本"作为**有原则规则**的落点(非硬编码 lwjgl 表):
/// - Fabric → `net.fabricmc.intermediary` equals mc
/// - Quilt  → `org.quiltmc.hashed` equals mc
///
/// 其它 loader(Forge/NeoForge/LiteLoader)的依赖由它们自己的版本文件携带
/// (`cached_requires`,如 `net.minecraft` equals mc),此处不内建。返回 `None` 表示
/// 该 uid 无内建 require。
fn loader_requires(uid: &str, mc_version: &str) -> Option<Vec<Require>> {
    match uid {
        "net.fabricmc.fabric-loader" => {
            Some(vec![Require::equals(UID_FABRIC_INTERMEDIARY, mc_version)])
        }
        "org.quiltmc.quilt-loader" => {
            Some(vec![Require::equals(UID_QUILT_HASHED, mc_version)])
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- serde round-trip ----

    #[test]
    fn pack_profile_roundtrip_with_serde_renames() {
        let mut pack = PackProfile::new();
        pack.components.push(Component::important(UID_MINECRAFT, Some("1.20.1".into())));
        let mut loader = Component::important("net.fabricmc.fabric-loader", Some("0.15.7".into()));
        loader.cached_name = Some("Fabric Loader".into());
        loader.cached_requires = vec![Require::equals(UID_FABRIC_INTERMEDIARY, "1.20.1")];
        pack.components.push(loader);
        let mut inter = Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into()));
        inter.cached_version = Some("1.20.1".into());
        pack.components.push(inter);

        let json = serde_json::to_string_pretty(&pack).unwrap();
        // 验证 JSON 键名走的是 camelCase / Prism 命名,而非 Rust 字段名。
        assert!(json.contains("\"formatVersion\""));
        assert!(json.contains("\"dependencyOnly\""));
        assert!(json.contains("\"cachedVolatile\""));
        assert!(json.contains("\"cachedRequires\""));
        assert!(json.contains("\"cachedName\""));
        // Require 的 equals 键名是 "equals" 而不是 "equals_version"。
        assert!(json.contains("\"equals\""));
        // 默认 false 的布尔不应出现(disabled 未置位)。
        assert!(!json.contains("\"disabled\""));

        let back: PackProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, pack);
    }

    #[test]
    fn volatile_alias_reads_both_keys_writes_cached_volatile() {
        // Prism bug-for-bug:输入 "volatile" 应被接受。
        let raw = r#"{
            "formatVersion": 1,
            "components": [
                {"uid":"org.lwjgl3","version":"3.3.1","dependencyOnly":true,"volatile":true}
            ]
        }"#;
        let pack: PackProfile = serde_json::from_str(raw).unwrap();
        assert!(pack.components[0].cached_volatile);
        assert!(pack.components[0].dependency_only);
        // 输出统一写 cachedVolatile。
        let json = serde_json::to_string(&pack).unwrap();
        assert!(json.contains("\"cachedVolatile\":true"));
        assert!(!json.contains("\"volatile\":"));
    }

    #[test]
    fn parses_minimal_pack() {
        let raw = r#"{"formatVersion":1,"components":[{"uid":"net.minecraft","version":"1.21","important":true}]}"#;
        let pack: PackProfile = serde_json::from_str(raw).unwrap();
        assert_eq!(pack.format_version, 1);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.minecraft_version(), Some("1.21"));
        assert!(pack.components[0].important);
    }

    // ---- set_component / get_component ----

    #[test]
    fn set_component_appends_then_updates() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.minecraft_version(), Some("1.20.1"));
        assert!(pack.get_component(UID_MINECRAFT).unwrap().important);

        // 再次 set 同 uid → 就地更新版本,不新增。
        pack.set_component(UID_MINECRAFT, "1.20.4", false);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.minecraft_version(), Some("1.20.4"));
        // important 已置位,不应被 false 复位。
        assert!(pack.get_component(UID_MINECRAFT).unwrap().important);

        // 不同 uid → 追加。
        pack.set_component("net.minecraftforge", "47.2.0", true);
        assert_eq!(pack.components.len(), 2);
        assert_eq!(pack.get_component("net.minecraftforge").unwrap().version.as_deref(), Some("47.2.0"));
    }

    // ---- detect_loader ----

    #[test]
    fn detect_loader_from_uid() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);

        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        assert_eq!(pack.detect_loader(), LoaderKind::Fabric);
        assert_eq!(
            pack.detect_loader_component().map(|c| c.uid.as_str()),
            Some("net.fabricmc.fabric-loader")
        );
    }

    #[test]
    fn detect_loader_neoforge_not_misread_as_forge() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.4", true);
        pack.set_component("net.neoforged", "20.4.237", true);
        // uid 一等公民:不会像子串猜测那样把 neoforged 误判成 forge。
        assert_eq!(pack.detect_loader(), LoaderKind::NeoForge);
    }

    #[test]
    fn intermediary_is_not_a_loader() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        pack.components.push(Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into())));
        // dependency_only 的 intermediary 不应被当作 loader。
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);
    }

    #[test]
    fn disabled_loader_not_detected() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        pack.get_component_mut("net.fabricmc.fabric-loader").unwrap().disabled = true;
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);
    }

    // ---- loader_uid / known_loader 表 ----

    #[test]
    fn loader_uid_table_maps_both_directions() {
        assert_eq!(loader_uid(LoaderKind::Fabric), Some("net.fabricmc.fabric-loader"));
        assert_eq!(loader_uid(LoaderKind::NeoForge), Some("net.neoforged"));
        assert_eq!(loader_uid(LoaderKind::Vanilla), None);
        assert_eq!(loader_uid(LoaderKind::OptiFine), None);

        assert_eq!(known_loader("net.neoforged").unwrap().kind, LoaderKind::NeoForge);
        assert!(known_loader("net.minecraft").is_none(), "vanilla 不是 loader");
        assert!(known_loader("org.lwjgl3").is_none(), "lwjgl 不是 loader");
    }

    // ---- loader_conflict ----

    #[test]
    fn detects_double_loader_conflict() {
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        assert!(pack.loader_conflict().is_none());
        pack.set_component("net.minecraftforge", "47.2.0", true);
        assert!(pack.loader_conflict().is_some());
    }

    // ---- from_loader ----

    #[test]
    fn from_loader_builds_two_component_pack() {
        let pack = PackProfile::from_loader("1.20.1", LoaderKind::Fabric, Some("0.15.7"));
        assert_eq!(pack.components.len(), 2);
        assert_eq!(pack.components[0].uid, UID_MINECRAFT);
        assert!(pack.components[0].important);
        assert_eq!(pack.components[1].uid, "net.fabricmc.fabric-loader");
        assert!(pack.components[1].important);
        assert_eq!(pack.detect_loader(), LoaderKind::Fabric);
    }

    #[test]
    fn from_loader_vanilla_is_single_component() {
        let pack = PackProfile::from_loader("1.21", LoaderKind::Vanilla, None);
        assert_eq!(pack.components.len(), 1);
        assert_eq!(pack.components[0].uid, UID_MINECRAFT);
        assert_eq!(pack.detect_loader(), LoaderKind::Vanilla);
    }

    // ---- resolver ----

    #[test]
    fn resolve_injects_fabric_intermediary_equal_to_mc() {
        let mut pack = PackProfile::from_loader("1.20.1", LoaderKind::Fabric, Some("0.15.7"));
        let changed = pack.resolve();
        assert!(changed, "首轮应注入 intermediary");

        // intermediary 被注入、版本 == mc、是 dependency_only。
        let inter = pack.get_component(UID_FABRIC_INTERMEDIARY).expect("应注入 intermediary");
        assert_eq!(inter.version.as_deref(), Some("1.20.1"));
        assert!(inter.dependency_only);

        // 顺序:intermediary 必须在 fabric-loader 之前(保证合并序)。
        let pos_inter = pack.components.iter().position(|c| c.uid == UID_FABRIC_INTERMEDIARY).unwrap();
        let pos_loader = pack.components.iter().position(|c| c.uid == "net.fabricmc.fabric-loader").unwrap();
        assert!(pos_inter < pos_loader, "intermediary 应夹在 vanilla 与 loader 之间");

        // 幂等:再次 resolve 不应再改动。
        assert!(!pack.resolve(), "已稳定,二次 resolve 不应改动");
    }

    #[test]
    fn resolve_injects_quilt_hashed() {
        let mut pack = PackProfile::from_loader("1.20.1", LoaderKind::Quilt, Some("0.20.0"));
        pack.resolve();
        let hashed = pack.get_component(UID_QUILT_HASHED).expect("应注入 hashed");
        assert_eq!(hashed.version.as_deref(), Some("1.20.1"));
    }

    #[test]
    fn resolve_corrects_stale_intermediary_version() {
        // intermediary 已存在但版本与 mc 不符(如改了 mc 版本)→ 解析器应改回 mc 版本。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.4", true);
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        // 故意放一个过期版本的 intermediary。
        let mut stale = Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into()));
        stale.cached_version = Some("1.20.1".into());
        pack.components.insert(0, stale);

        pack.resolve();
        assert_eq!(
            pack.get_component(UID_FABRIC_INTERMEDIARY).unwrap().version.as_deref(),
            Some("1.20.4"),
            "改 mc 版本后 intermediary 应级联到新版本"
        );
    }

    #[test]
    fn resolve_removes_orphaned_volatile_dependency() {
        // 一个 volatile dependency_only 组件,但无人依赖它 → 应被移除。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        // 无 loader,intermediary 没有依赖者。
        pack.components.push(Component::dependency(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into())));
        assert!(pack.has_component(UID_FABRIC_INTERMEDIARY));

        pack.resolve();
        assert!(
            !pack.has_component(UID_FABRIC_INTERMEDIARY),
            "无人依赖的 volatile 依赖项应被平凡移除"
        );
    }

    #[test]
    fn resolve_keeps_non_volatile_dependency() {
        // 非 volatile 的 dependency_only 不应被自动移除(用户可能显式保留)。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        let mut dep = Component::new(UID_FABRIC_INTERMEDIARY, Some("1.20.1".into()));
        dep.dependency_only = true;
        dep.cached_volatile = false;
        pack.components.push(dep);

        pack.resolve();
        assert!(pack.has_component(UID_FABRIC_INTERMEDIARY), "非 volatile 依赖项应保留");
    }

    #[test]
    fn resolve_without_minecraft_anchor_is_noop() {
        let mut pack = PackProfile::new();
        pack.set_component("net.fabricmc.fabric-loader", "0.15.7", true);
        assert!(!pack.resolve(), "无 net.minecraft 锚版本时无可解析");
    }

    #[test]
    fn resolve_respects_cached_requires_from_version_file() {
        // Forge 的版本文件携带 net.minecraft==mc 的 cached_requires;mc 已存在则不注入,
        // 版本一致则不改动 → 稳定。
        let mut pack = PackProfile::new();
        pack.set_component(UID_MINECRAFT, "1.20.1", true);
        let mut forge = Component::important("net.minecraftforge", Some("47.2.0".into()));
        forge.cached_requires = vec![Require::equals(UID_MINECRAFT, "1.20.1")];
        pack.components.push(forge);

        assert!(!pack.resolve(), "mc 已满足 forge 的 equals 约束,应稳定无改动");
        assert_eq!(pack.minecraft_version(), Some("1.20.1"));
    }
}
