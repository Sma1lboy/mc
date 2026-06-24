//! 内容平台(Mod / 整合包 / 资源包 / 光影)聚合层。
//!
//! 这里定义一套**与具体平台无关**的统一数据模型(`SearchHit` /
//! `ProjectVersion` / `VersionFile` / `Dependency`),各平台后端(目前只有
//! Modrinth)负责把自家 API 的 json 映射到这套模型上。上层(实例 / UI)只看
//! 这套模型,从而可以在未来无痛接入 CurseForge 等其它源。
//!
//! 设计取舍:
//! - 不引入 `async_trait`(不加依赖),因此**不**定义统一 trait,而是让每个
//!   后端导出一个具体 struct(如 [`modrinth::ModrinthApi`]),方法签名保持一致。
//! - 所有模型都派生 `Serialize`,方便直接经 Tauri command 回传给前端;同时派生
//!   `Deserialize` 以便测试/缓存,但**不**直接对平台原始 json 反序列化——
//!   平台字段名各异,映射在各后端模块里手写完成。

pub mod curseforge;
pub mod dependency;
pub mod modrinth;
pub mod provider;

use serde::{Deserialize, Serialize};

/// 一个实例 loader 实际能加载的 mod loader 集合(全小写)。
///
/// Quilt 在设计上向后兼容 Fabric:绝大多数 Fabric mod 能直接在 Quilt 上运行,而很多
/// 项目只发布 `fabric` 版本。因此 Quilt 实例搜索/安装/查更新时应**同时**接受 `fabric`,
/// 否则整个 Fabric 生态对 Quilt 用户都不可见。其余 loader 只接受自身,行为与之前完全一致。
///
/// 输入按 ASCII 小写归一;空串返回空集(调用方据此视为"不按 loader 过滤")。
pub fn accepted_loaders(loader: &str) -> Vec<String> {
    match loader.trim().to_ascii_lowercase().as_str() {
        "" => Vec::new(),
        "quilt" => vec!["quilt".to_string(), "fabric".to_string()],
        other => vec![other.to_string()],
    }
}

/// 资源类型。对应 Modrinth 的 `project_type` 取值。
///
/// 注意:Modrinth 把"数据包"也归在 `mod` 类型下(用 category `datapack` 区分),
/// 但为了上层语义清晰我们仍单列 [`ResourceKind::Datapack`],并在 facets 里转成
/// 合适的查询。`as_modrinth_project_type` 给出实际用于 `project_type` facet 的值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Mod,
    Modpack,
    ResourcePack,
    Shader,
    Datapack,
}

impl ResourceKind {
    /// 映射到 Modrinth `project_type` facet 的字符串值。
    ///
    /// Modrinth 没有独立的 `datapack` project_type——数据包以 `mod` 项目存在、
    /// 通过 category 标记,故这里回退到 `mod`(再由调用方追加 `categories:datapack`)。
    pub fn as_modrinth_project_type(self) -> &'static str {
        match self {
            ResourceKind::Mod => "mod",
            ResourceKind::Modpack => "modpack",
            ResourceKind::ResourcePack => "resourcepack",
            ResourceKind::Shader => "shader",
            ResourceKind::Datapack => "mod",
        }
    }
}

/// 搜索结果中的一个项目(或 `get_project` 的精简视图)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SearchHit {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub author: String,
    pub downloads: u64,
    pub icon_url: Option<String>,
    /// High-res landscape cover (Modrinth gallery / featured image). Preferred
    /// over `icon_url` for card covers; the small square icon looks low-res when
    /// upscaled to fill a 16:9 card.
    #[serde(default)]
    pub gallery_url: Option<String>,
    pub categories: Vec<String>,
}

/// 一个项目的某个具体版本(release)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectVersion {
    pub id: String,
    pub name: String,
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<VersionFile>,
    pub dependencies: Vec<Dependency>,
}

impl ProjectVersion {
    /// 取该版本的"主文件"(`primary == true`),若都不是 primary 则取第一个。
    /// 下载时通常只需要主文件,这是个便捷入口。
    pub fn primary_file(&self) -> Option<&VersionFile> {
        self.files
            .iter()
            .find(|f| f.primary)
            .or_else(|| self.files.first())
    }
}

/// 版本下的一个可下载文件。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    pub sha1: Option<String>,
    /// sha512(Modrinth 提供;整合包导入/导出反查需要)。
    #[serde(default)]
    pub sha512: Option<String>,
    pub size: Option<u64>,
    pub primary: bool,
}

/// 一个版本对其它项目/版本的依赖关系。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    /// 取值如 `required` / `optional` / `incompatible` / `embedded`。
    pub dependency_type: String,
}

// ===========================================================================
// Provider 抽象的共享类型(见 provider.rs / curseforge.rs / dependency.rs)
// ===========================================================================

/// 内容平台标识。跨平台统一身份 = `(ProviderId, project_id, version_id)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Modrinth,
    CurseForge,
}

/// 平台支持的哈希算法(声明在 [`ProviderCaps::hash_algos`],按反查偏好序)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgo {
    Sha1,
    Sha512,
    Md5,
    Murmur2,
}

/// 平台能力声明。引擎据此选取反查算法、决定是否需要 API key。
#[derive(Debug, Clone)]
pub struct ProviderCaps {
    pub id: ProviderId,
    pub readable_name: &'static str,
    /// 可反查的哈希算法,按偏好序(Modrinth `[Sha512,Sha1]`;CurseForge `[Sha1,Md5,Murmur2]`)。
    pub hash_algos: &'static [HashAlgo],
    /// 是否需要 API key(CurseForge 需要,Modrinth 不需要)。
    pub needs_api_key: bool,
}

/// 排序方式(统一枚举,各平台映射到自家参数)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMethod {
    #[default]
    Relevance,
    Downloads,
    Newest,
    Updated,
}

/// 统一搜索查询参数。
///
/// `game_version` / `loader` 是**单值兼容入口**:嵌入式 ContentBrowser 仍按当前实例的
/// 兼容性传单个游戏版本 / loader 过滤。新增的 `game_versions` / `loaders` / `categories` /
/// `environment` 是 Discover 多选 facet 入口——两者并存,构造 Modrinth facets 时取并集
/// (见 [`modrinth::build_facets`])。CurseForge 只消费单值 `game_version` / `loader`。
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub text: String,
    pub kind: ResourceKind,
    pub game_version: Option<String>,
    pub loader: Option<String>,
    /// 多选游戏版本(Modrinth `versions:` OR 组)。与 `game_version` 合并去重。
    pub game_versions: Vec<String>,
    /// 多选 loader(Modrinth `categories:` OR 组)。与 `loader` 合并去重(经 `accepted_loaders`)。
    pub loaders: Vec<String>,
    /// 多选内容分类(Modrinth `categories:` —— 每个分类各成一个 AND 组)。
    pub categories: Vec<String>,
    /// 运行环境过滤:`"client"` / `"server"`(其余忽略)。Modrinth `client_side` / `server_side` facet。
    pub environment: Option<String>,
    /// 仅开源项目(Modrinth `open_source:true` facet);`None`/`Some(false)` = 不过滤。
    pub open_source: Option<bool>,
    pub offset: u32,
    pub limit: u32,
    pub sort: SortMethod,
}

impl SearchQuery {
    /// 便捷构造:仅文本 + 类型,其余默认(offset 0、limit 20、按相关度)。
    pub fn new(text: impl Into<String>, kind: ResourceKind) -> Self {
        Self {
            text: text.into(),
            kind,
            game_version: None,
            loader: None,
            game_versions: Vec::new(),
            loaders: Vec::new(),
            categories: Vec::new(),
            environment: None,
            open_source: None,
            offset: 0,
            limit: 20,
            sort: SortMethod::default(),
        }
    }
}

/// 一个"本地文件 -> 远程 project/version"的解析结果。整合包导入(id→URL)与
/// 导出(hash→引用)都产出它;`(provider, project_id, version_id)` 是去重键。
#[derive(Debug, Clone)]
pub struct ResolvedFile {
    pub provider: ProviderId,
    pub project_id: String,
    pub version_id: String,
    /// 可下载文件(复用 [`VersionFile`]:url/filename/sha1/sha512/size/primary)。
    pub file: VersionFile,
    pub project_name: Option<String>,
    pub project_slug: Option<String>,
    pub authors: Vec<String>,
}

#[cfg(test)]
mod accepted_loaders_tests {
    use super::accepted_loaders;

    #[test]
    fn quilt_also_accepts_fabric() {
        assert_eq!(accepted_loaders("quilt"), vec!["quilt", "fabric"]);
        // 大小写归一。
        assert_eq!(accepted_loaders("Quilt"), vec!["quilt", "fabric"]);
    }

    #[test]
    fn other_loaders_accept_only_self() {
        assert_eq!(accepted_loaders("fabric"), vec!["fabric"]);
        assert_eq!(accepted_loaders("forge"), vec!["forge"]);
        assert_eq!(accepted_loaders("neoforge"), vec!["neoforge"]);
    }

    #[test]
    fn empty_is_empty() {
        assert!(accepted_loaders("").is_empty());
        assert!(accepted_loaders("  ").is_empty());
    }
}
