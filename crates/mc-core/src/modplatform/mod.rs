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

pub mod modrinth;

use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    pub sha1: Option<String>,
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
