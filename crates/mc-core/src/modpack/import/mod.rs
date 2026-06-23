//! 可插拔整合包**导入核心**:统一接口 + 与来源无关的中间模型 + 注册表分发。
//!
//! 见 `docs/modules/modpack-import.md`。架构语义照搬 Prism 的「一个固定引擎 + 少量
//! 每格式扩展点 + 优先级嗅探」,但拆成可单测的纯函数:
//!
//! - [`ModpackImporter`][]:每种格式实现一份(`detect`/`plan`/`resolve`),**唯一懂该格式
//!   schema 的地方**。新增格式 = 写一个实现 + 在 [`engine::ImportEngine::with_defaults`]
//!   里按优先级加一行,引擎零改动。
//! - [`ArchiveIndex`]:`detect()` 的只读归档视图,让 importer 脱离具体 zip 类型,可用
//!   假对象做单测(见 [`super::import::tests`])。
//! - [`ImportPlan`]:所有格式 `plan()` 都产出**同一个**中间模型;引擎此后什么都不再需要
//!   知道格式。这是关键的缝。
//!
//! **async 约定**:沿用仓库「不引 `async_trait`」的决定(见 `modplatform/provider.rs`)——
//! `detect()` / `plan()` 同步;只有 `resolve()` 联网,用 [`futures::future::BoxFuture`]。

pub mod archive;
pub mod curseforge;
pub mod engine;
pub mod mcbbs;
pub mod modrinth;
pub mod multimc;

#[cfg(test)]
mod tests;

use std::path::Path;

use futures::future::BoxFuture;

use mc_types::LoaderKind;

use crate::download::Downloader;
use crate::error::Result;
use crate::modplatform::provider::ProviderRegistry;

pub use engine::{ImportEngine, ImportOptions, ImportOutcome, ImportSource};

// ===========================================================================
// 统一接口
// ===========================================================================

/// 一种整合包格式的导入器。对象安全(`Box<dyn ModpackImporter>` 可入引擎注册表)。
pub trait ModpackImporter: Send + Sync {
    /// 稳定 id,也是 [`DetectMatch::format`] 的取值(如 `"modrinth"`)。
    fn id(&self) -> &'static str;

    /// 只读嗅探(对应 Prism 的 `detectInstance` lambda):在已打开的归档索引里找本格式的
    /// 标记文件,命中则报告包根([`DetectMatch::archive_root`])。**不得**解压 / 下载。
    ///
    /// 返回 [`DetectMatch::confidence`](而非 bool):让根级标记得分高于深层标记,使
    /// 引擎在多 importer 命中时取最高分(平局按注册序)。
    fn detect(&self, archive: &dyn ArchiveIndex) -> Option<DetectMatch>;

    /// 解析本格式已解压到 `staging` 的 manifest 为与来源无关的 [`ImportPlan`]。
    ///
    /// **纯函数式**:只读 `staging`、不联网、不改实例 —— 等价 Prism 的
    /// `parseManifest` / `loadManifest`,可对着 fixture manifest 单测。`m` 是本格式 `detect()`
    /// 的产物(携带包根等信息,但 `staging` 已是按包根解压后的子树,故 `plan` 内一般从
    /// `staging` 根读标记文件)。
    fn plan(&self, staging: &Path, m: &DetectMatch) -> Result<ImportPlan>;

    /// 可选第二趟:把 [`ImportPlan::unresolved`] 的 id 引用解析成具体下载源,并收集
    /// 无第三方下载链接的 [`BlockedFile`]。默认空操作。
    ///
    /// 只有「给的是 id 而非 URL」的格式(CurseForge、部分 MCBBS)覆盖它,委托
    /// `registry` 里的 [`crate::modplatform::provider::ResourceProvider`] 批量查文件。
    /// `dl` 供需要直接抓取的实现复用同一连接池。
    fn resolve<'a>(
        &'a self,
        _dl: &'a Downloader,
        _registry: &'a ProviderRegistry,
        _plan: &'a mut ImportPlan,
    ) -> BoxFuture<'a, Result<Vec<BlockedFile>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// `detect()` 用的归档只读视图。
///
/// 让 importer 脱离具体 zip 类型:真实导入时由 [`archive::ZipArchiveIndex`] 实现,单测时
/// 可用一个内存假对象(见 tests)。`entries` 是所有**文件**条目的相对路径(`/` 分隔);
/// `read_small` 供 CF / MCBBS 读 `manifest.json` 内容做内容判别。
pub trait ArchiveIndex {
    /// 归档内所有文件条目的相对路径(`/` 分隔,不含目录条目)。
    fn entries(&self) -> &[String];

    /// 按条目名读取一个**小**文件的字节(供内容判别);读不到返回 `None`。
    fn read_small(&self, name: &str) -> Option<Vec<u8>>;
}

// ===========================================================================
// 归档路径小工具
// ---------------------------------------------------------------------------
// 四个 importer adapter 此前各自重定义了一份 basename/depth(字节级相同),
// mcbbs/multimc 还各重定义了 shallowest_marker。统一到这里,改一处全生效。
// ===========================================================================

/// 条目的 basename(最后一个 `/` 之后;无 `/` 时即原串)。
pub(crate) fn basename(entry: &str) -> &str {
    entry.rsplit('/').next().unwrap_or(entry)
}

/// 条目的路径深度(非空路径段数)。
pub(crate) fn depth(entry: &str) -> usize {
    entry.split('/').filter(|s| !s.is_empty()).count()
}

/// 在归档里找 basename == `name` 的**最浅**条目(在嵌套包里定位标记文件)。
pub(crate) fn shallowest_marker(archive: &dyn ArchiveIndex, name: &str) -> Option<String> {
    archive
        .entries()
        .iter()
        .filter(|e| basename(e) == name)
        .min_by_key(|e| depth(e))
        .cloned()
}

// ===========================================================================
// 与来源无关的中间模型
// ===========================================================================

/// `detect()` 的命中:格式 id + 包在归档内的真实根 + 置信度。
///
/// `archive_root` 让探测与解压解耦:MultiMC 嵌套目录 / Technic 映射到 `minecraft` 时,
/// 引擎按它把对应子树解压到 staging。空串表示包根即归档根。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectMatch {
    /// 命中的格式 id(== [`ModpackImporter::id`])。
    pub format: String,
    /// 包在归档内的真实根(相对路径,`/` 分隔;空串 = 归档根)。
    pub archive_root: String,
    /// 置信度(根级标记应高于深层标记);引擎取最高分,平局按注册序。
    pub confidence: u32,
}

impl DetectMatch {
    /// 由标记条目的相对路径推出包根并构造命中。
    ///
    /// 包根 = 标记条目的父目录(去掉 basename);置信度按根深度递减(越浅越可信),
    /// 这样根级标记永远胜过被 `overrides/` 等深层目录裹住的同名文件。
    pub fn from_marker(format: &str, marker_path: &str) -> Self {
        let root = parent_dir(marker_path);
        // 根越浅置信度越高(深度 0 → 1000,每深一层 -1)。
        let depth = depth(&root) as u32;
        DetectMatch {
            format: format.to_string(),
            archive_root: root,
            confidence: 1000u32.saturating_sub(depth),
        }
    }
}

/// 取一个 `/` 分隔路径的父目录(无父目录则空串)。
fn parent_dir(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

/// 与来源无关的导入计划。每种格式的 `plan()` 都产出它;引擎据此执行全部副作用。
#[derive(Debug, Clone, PartialEq)]
pub struct ImportPlan {
    /// 整合包名(写进实例 `instance.json` 的 name)。
    pub pack_name: String,
    /// 整合包版本号(自由文本),无则 `None`。
    pub pack_version: Option<String>,
    /// 目标 Minecraft 原版版本。
    pub mc_version: String,
    /// loader 家族 + 版本;`None` 表示原版。
    pub loader: Option<(LoaderKind, String)>,
    /// 自带下载源的受管理文件(mrpack / packwiz);引擎直接下。
    pub files: Vec<PlannedFile>,
    /// 仅给了 id、需 `resolve()` 二次解析的引用(CurseForge);mrpack / multimc 为空。
    pub unresolved: Vec<UnresolvedRef>,
    /// 顺序拷进游戏目录的 override 根:mrpack=`["overrides","client-overrides"]`、
    /// CF/MCBBS=`["overrides"]`、MMC=`[".minecraft"]`。引擎逐个经 `safe_join` 铺设。
    pub override_roots: Vec<String>,
    /// 推荐分配内存(MiB),写进实例配置;无则保留默认。
    pub recommended_ram_mib: Option<u64>,
    /// 整合包指定的附加 JVM 参数(MCBBS `launchInfo.javaArgument`);写进实例配置的
    /// `jvm_args`。其它格式留空。**不**含 Java 路径 / 启动命令(那些不可自动信任)。
    pub extra_jvm_args: Vec<String>,
    /// 整合包指定的附加游戏参数(MCBBS `launchInfo.launchArgument`);写进实例配置的
    /// `game_args`。其它格式留空。
    pub extra_game_args: Vec<String>,
    /// 「这实例来自哪个平台的哪个包/版本」溯源,供日后「更新整合包」。
    pub managed: Option<ManagedPack>,
}

impl ImportPlan {
    /// 构造一个最小计划(仅 name + mc_version),其余字段空。
    pub fn new(pack_name: impl Into<String>, mc_version: impl Into<String>) -> Self {
        ImportPlan {
            pack_name: pack_name.into(),
            pack_version: None,
            mc_version: mc_version.into(),
            loader: None,
            files: Vec::new(),
            unresolved: Vec::new(),
            override_roots: Vec::new(),
            recommended_ram_mib: None,
            extra_jvm_args: Vec::new(),
            extra_game_args: Vec::new(),
            managed: None,
        }
    }
}

/// 一个有明确下载源的受管理文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// 相对游戏目录的落盘路径,如 `mods/sodium.jar`;引擎用前必过 `safe_join`。
    pub rel_path: String,
    /// 有序候选 URL(多源!)——引擎依次尝试,主源失败回退。
    pub sources: Vec<String>,
    /// 期望 sha1(部分格式提供)。
    pub sha1: Option<String>,
    /// 期望 sha512(mrpack 的规范哈希)。
    pub sha512: Option<String>,
    /// 期望大小(字节)。
    pub size: Option<u64>,
    /// 是否必备:可选项可跳过 / 落 `.disabled`。
    pub required: bool,
}

/// 需要二次解析才有 URL 的引用(CurseForge `projectID`/`fileID`)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedRef {
    pub project_id: String,
    pub file_id: String,
    /// 落盘相对目录(如 `mods`),解析出文件名后拼成完整 rel_path。
    pub target_dir: String,
    pub required: bool,
}

/// 无第三方下载链接的 CF 文件(法律上不可再分发)——回传给 UI 让用户手动下,引擎跳过它。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedFile {
    pub name: String,
    pub website_url: String,
    pub target_dir: String,
    pub required: bool,
}

/// 跨格式「这实例来自哪个平台的哪个包/版本」的溯源记录(MultiMC 的 `ManagedPack*` 是它的鼻祖)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedPack {
    pub platform: String,
    pub project_id: String,
    pub version_id: Option<String>,
}

#[cfg(test)]
mod model_tests {
    use super::*;

    #[test]
    fn detect_match_from_root_marker_is_most_confident() {
        let m = DetectMatch::from_marker("modrinth", "modrinth.index.json");
        assert_eq!(m.format, "modrinth");
        assert_eq!(m.archive_root, "");
        assert_eq!(m.confidence, 1000);
    }

    #[test]
    fn detect_match_nested_marker_lowers_confidence_and_keeps_root() {
        let m = DetectMatch::from_marker("modrinth", "MyPack/modrinth.index.json");
        assert_eq!(m.archive_root, "MyPack");
        assert_eq!(m.confidence, 999, "嵌套一层应比根级低 1");

        let deeper = DetectMatch::from_marker("multimc", "a/b/c/mmc-pack.json");
        assert_eq!(deeper.archive_root, "a/b/c");
        assert_eq!(deeper.confidence, 997);
    }

    #[test]
    fn parent_dir_of_root_file_is_empty() {
        assert_eq!(parent_dir("manifest.json"), "");
        assert_eq!(parent_dir("Pack/manifest.json"), "Pack");
        assert_eq!(parent_dir("a/b/c.json"), "a/b");
    }
}
