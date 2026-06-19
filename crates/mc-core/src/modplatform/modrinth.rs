//! Modrinth API v2 后端(开放、无需 API key)。
//!
//! 文档:<https://docs.modrinth.com/api/>。这里只用到三个只读端点:
//! - `GET /v2/search`            —— 搜索项目,facets 过滤 project_type/版本/loader
//! - `GET /v2/project/{id}/version` —— 列项目版本,可按 loaders/game_versions 过滤
//! - `GET /v2/project/{id}`      —— 取单个项目元信息
//!
//! 设计要点:
//! - **自带 `reqwest::Client`**。Modrinth 要求带一个能联系到作者的 User-Agent
//!   (否则可能限流/封禁),所以我们硬编码一个含仓库地址的 UA。
//! - 平台原始 json 的字段名(`project_type` / `version_number` / `game_versions`
//!   等)与统一模型不同,故这里用一组 `Raw*` 内部类型承接原始 json,再由纯映射
//!   函数转成 [`crate::modplatform`] 的统一模型。映射函数无 IO、可单测。
//! - 容错:缺字段一律走 `#[serde(default)]` 给默认值,不让单个字段缺失把整次请
//!   求打挂。HTTP/网络错误映射成 [`CoreError::Network`],反序列化错误映射成
//!   [`CoreError::Parse`]。

use serde::Deserialize;

use crate::error::{CoreError, Result};

use super::{Dependency, ProjectVersion, ResourceKind, SearchHit, VersionFile};

/// Modrinth API v2 根地址。
const API_BASE: &str = "https://api.modrinth.com/v2";

/// Modrinth 要求的 User-Agent(含联系方式形式)。
const USER_AGENT: &str = "mc-launcher/0.1 (github.com/sma1lboy/mc-launcher)";

/// Modrinth 后端客户端。`new()` 自带一个配置好 UA 的 `reqwest::Client`。
#[derive(Debug, Clone)]
pub struct ModrinthApi {
    client: reqwest::Client,
    base: String,
}

impl Default for ModrinthApi {
    fn default() -> Self {
        Self::new()
    }
}

impl ModrinthApi {
    /// 构造一个新客户端。UA 在此固化;构造失败(几乎不会)会 panic,因为没有
    /// 任何运行时输入能让它失败——这是纯静态配置。
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            // reqwest 仅在 TLS 后端初始化失败时报错,属于环境级灾难;此处用静态
            // 配置,失败即代表整个进程无法发任何请求,直接 expect 暴露问题。
            .expect("failed to build reqwest client for Modrinth");
        Self { client, base: API_BASE.to_string() }
    }

    /// 用自定义 base url 构造(主要给测试/镜像用)。
    pub fn with_base(base: impl Into<String>) -> Self {
        let mut api = Self::new();
        api.base = base.into();
        api
    }

    /// 搜索项目。
    ///
    /// - `kind`:资源类型,转成 `project_type` facet。
    /// - `game_version`:可选,转成 `versions:<v>` facet。
    /// - `loader`:可选,Modrinth 把 loader 放在 categories 维度,转成
    ///   `categories:<loader>` facet。
    /// - `limit`:返回条数上限(Modrinth 默认 10,最大 100,这里夹到 [1,100])。
    ///
    /// facets 是一个"AND of OR"结构的二维数组,详见 Modrinth 文档。
    pub async fn search(
        &self,
        query: &str,
        kind: ResourceKind,
        game_version: Option<&str>,
        loader: Option<&str>,
        limit: u32,
    ) -> Result<Vec<SearchHit>> {
        let facets = build_facets(kind, game_version, loader);
        let limit = limit.clamp(1, 100);

        let url = format!("{}/search", self.base);
        let resp: RawSearchResponse = self
            .client
            .get(&url)
            .query(&[
                ("query", query),
                ("facets", facets.as_str()),
                ("limit", &limit.to_string()),
                ("index", "relevance"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp.hits.into_iter().map(map_search_hit).collect())
    }

    /// 列出某项目的所有版本,可按游戏版本 / loader 过滤。
    ///
    /// Modrinth 的过滤参数是 json 编码的字符串数组,例如
    /// `loaders=["fabric"]&game_versions=["1.20.1"]`。
    pub async fn get_versions(
        &self,
        project_id: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ProjectVersion>> {
        let url = format!("{}/project/{}/version", self.base, project_id);

        // query 的 value 需是 json 数组字符串。用 to_owned 持有,使引用活到请求结束。
        let loaders_param = loader.map(|l| json_string_array(&[l]));
        let versions_param = game_version.map(|g| json_string_array(&[g]));

        let mut req = self.client.get(&url);
        if let Some(ref l) = loaders_param {
            req = req.query(&[("loaders", l.as_str())]);
        }
        if let Some(ref g) = versions_param {
            req = req.query(&[("game_versions", g.as_str())]);
        }

        let raws: Vec<RawVersion> =
            req.send().await?.error_for_status()?.json().await?;

        Ok(raws.into_iter().map(map_version).collect())
    }

    /// 取单个项目的元信息,映射成精简的 [`SearchHit`]。
    pub async fn get_project(&self, id: &str) -> Result<SearchHit> {
        let url = format!("{}/project/{}", self.base, id);
        let raw: RawProject = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(map_project(raw))
    }

    /// 便捷方法:从已拿到的字节做搜索响应反序列化(主要用于把 reqwest 之外的
    /// 字节流接进来,或测试)。失败映射成 [`CoreError::Parse`]。
    pub fn parse_search_response(bytes: &[u8]) -> Result<Vec<SearchHit>> {
        let resp: RawSearchResponse = serde_json::from_slice(bytes)
            .map_err(|e| CoreError::Parse { what: "modrinth search response".into(), source: e })?;
        Ok(resp.hits.into_iter().map(map_search_hit).collect())
    }

    /// 便捷方法:解析 `/project/{id}/version` 的版本数组字节。
    pub fn parse_versions(bytes: &[u8]) -> Result<Vec<ProjectVersion>> {
        let raws: Vec<RawVersion> = serde_json::from_slice(bytes)
            .map_err(|e| CoreError::Parse { what: "modrinth versions".into(), source: e })?;
        Ok(raws.into_iter().map(map_version).collect())
    }

    /// 便捷方法:解析 `/project/{id}` 的项目对象字节。
    pub fn parse_project(bytes: &[u8]) -> Result<SearchHit> {
        let raw: RawProject = serde_json::from_slice(bytes)
            .map_err(|e| CoreError::Parse { what: "modrinth project".into(), source: e })?;
        Ok(map_project(raw))
    }
}

// ============================ facets / query 构造 ============================

/// 构造 Modrinth `facets` 参数(一个 json 字符串)。
///
/// facets 形如 `[["project_type:mod"],["versions:1.20.1"],["categories:fabric"]]`,
/// 外层数组各元素之间是 AND,内层数组各元素之间是 OR。我们每个维度只放一个值,
/// 因此内层都是单元素。
///
/// 数据包(`ResourceKind::Datapack`)在 Modrinth 是 `mod` 项目 + `datapack`
/// category,故额外追加 `categories:datapack`。
fn build_facets(kind: ResourceKind, game_version: Option<&str>, loader: Option<&str>) -> String {
    let mut groups: Vec<String> = Vec::new();

    groups.push(facet_group(&[&format!("project_type:{}", kind.as_modrinth_project_type())]));

    if matches!(kind, ResourceKind::Datapack) {
        groups.push(facet_group(&["categories:datapack"]));
    }

    if let Some(v) = game_version {
        groups.push(facet_group(&[&format!("versions:{v}")]));
    }
    if let Some(l) = loader {
        groups.push(facet_group(&[&format!("categories:{l}")]));
    }

    format!("[{}]", groups.join(","))
}

/// 把一组 facet 字符串拼成内层 OR 组,如 `["a:b","c:d"]`,每项做 json 字符串转义。
fn facet_group(items: &[&str]) -> String {
    let inner: Vec<String> = items.iter().map(|s| json_quote(s)).collect();
    format!("[{}]", inner.join(","))
}

/// 把一组字符串编码成 json 数组字符串,如 `["fabric"]`,用于 loaders/game_versions 参数。
fn json_string_array(items: &[&str]) -> String {
    let inner: Vec<String> = items.iter().map(|s| json_quote(s)).collect();
    format!("[{}]", inner.join(","))
}

/// 用 serde_json 给单个字符串做带引号的 json 转义(保证特殊字符安全)。
fn json_quote(s: &str) -> String {
    // serde_json::to_string 对 &str 永不失败(字符串总能序列化),unwrap 安全。
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

// ============================ 原始 json 承接类型 ============================

/// `/v2/search` 的顶层响应。
#[derive(Debug, Deserialize)]
struct RawSearchResponse {
    #[serde(default)]
    hits: Vec<RawSearchHit>,
}

/// 搜索结果中的一条 hit。字段名遵循 Modrinth `search` 端点(注意它和
/// `project` 端点字段不完全一样:这里 id 叫 `project_id`,作者叫 `author`)。
#[derive(Debug, Deserialize)]
struct RawSearchHit {
    #[serde(default)]
    project_id: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    icon_url: Option<String>,
    /// Modrinth search hits carry the featured gallery image and the full gallery
    /// URL list; either gives a high-res landscape cover.
    #[serde(default)]
    featured_gallery: Option<String>,
    #[serde(default)]
    gallery: Vec<String>,
    #[serde(default)]
    categories: Vec<String>,
}

/// `/v2/project/{id}` 的项目对象。这里 id 字段就叫 `id`,且**没有** `author`
/// 字段(作者要另算端点),所以作者留空。
#[derive(Debug, Deserialize)]
struct RawProject {
    #[serde(default)]
    id: String,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    icon_url: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
}

/// `/v2/project/{id}/version` 数组里的一个版本。
#[derive(Debug, Deserialize)]
struct RawVersion {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version_number: String,
    #[serde(default)]
    game_versions: Vec<String>,
    #[serde(default)]
    loaders: Vec<String>,
    #[serde(default)]
    files: Vec<RawFile>,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
}

#[derive(Debug, Deserialize)]
struct RawFile {
    #[serde(default)]
    url: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    hashes: RawHashes,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    primary: bool,
}

/// Modrinth 把校验和放在 `hashes` 对象里(`sha1` / `sha512`)。
#[derive(Debug, Default, Deserialize)]
struct RawHashes {
    #[serde(default)]
    sha1: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawDependency {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    version_id: Option<String>,
    #[serde(default)]
    dependency_type: Option<String>,
}

// ============================ 纯映射函数(可单测) ============================

fn map_search_hit(r: RawSearchHit) -> SearchHit {
    SearchHit {
        id: r.project_id,
        slug: r.slug,
        title: r.title,
        description: r.description,
        author: r.author,
        downloads: r.downloads,
        icon_url: r.icon_url,
        // 优先 featured gallery,否则取 gallery 第一张作高清封面。
        gallery_url: r.featured_gallery.or_else(|| r.gallery.into_iter().next()),
        categories: r.categories,
    }
}

fn map_project(r: RawProject) -> SearchHit {
    SearchHit {
        id: r.id,
        slug: r.slug,
        title: r.title,
        description: r.description,
        // /project 端点不带 author,留空由上层决定是否再查。
        author: String::new(),
        downloads: r.downloads,
        icon_url: r.icon_url,
        gallery_url: None,
        categories: r.categories,
    }
}

fn map_version(r: RawVersion) -> ProjectVersion {
    ProjectVersion {
        id: r.id,
        name: r.name,
        version_number: r.version_number,
        game_versions: r.game_versions,
        loaders: r.loaders,
        files: r.files.into_iter().map(map_file).collect(),
        dependencies: r.dependencies.into_iter().map(map_dependency).collect(),
    }
}

fn map_file(r: RawFile) -> VersionFile {
    VersionFile {
        url: r.url,
        filename: r.filename,
        sha1: r.hashes.sha1,
        size: r.size,
        primary: r.primary,
    }
}

fn map_dependency(r: RawDependency) -> Dependency {
    Dependency {
        project_id: r.project_id,
        version_id: r.version_id,
        // 缺省给 "required",这是 Modrinth 最常见且语义最保守的取值。
        dependency_type: r.dependency_type.unwrap_or_else(|| "required".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facets_only_kind() {
        let f = build_facets(ResourceKind::Mod, None, None);
        assert_eq!(f, r#"[["project_type:mod"]]"#);
    }

    #[test]
    fn facets_with_version_and_loader() {
        let f = build_facets(ResourceKind::Mod, Some("1.20.1"), Some("fabric"));
        assert_eq!(
            f,
            r#"[["project_type:mod"],["versions:1.20.1"],["categories:fabric"]]"#
        );
    }

    #[test]
    fn facets_datapack_adds_category() {
        // 数据包 → project_type:mod + categories:datapack
        let f = build_facets(ResourceKind::Datapack, None, None);
        assert_eq!(f, r#"[["project_type:mod"],["categories:datapack"]]"#);
    }

    #[test]
    fn facets_resourcepack_and_shader_type() {
        assert_eq!(
            build_facets(ResourceKind::ResourcePack, None, None),
            r#"[["project_type:resourcepack"]]"#
        );
        assert_eq!(
            build_facets(ResourceKind::Shader, None, None),
            r#"[["project_type:shader"]]"#
        );
    }

    #[test]
    fn json_string_array_encodes() {
        assert_eq!(json_string_array(&["fabric"]), r#"["fabric"]"#);
        assert_eq!(json_string_array(&["a", "b"]), r#"["a","b"]"#);
    }

    #[test]
    fn parse_search_response_maps_fields() {
        // 内联样本:覆盖字段重命名(project_id→id)与缺字段容错(无 icon_url)。
        let sample = r#"{
            "hits": [
                {
                    "project_id": "AABBCCDD",
                    "slug": "sodium",
                    "title": "Sodium",
                    "description": "A rendering engine",
                    "author": "jellysquid3",
                    "downloads": 12345,
                    "categories": ["optimization", "fabric"]
                }
            ],
            "total_hits": 1
        }"#;
        let hits = ModrinthApi::parse_search_response(sample.as_bytes()).unwrap();
        assert_eq!(hits.len(), 1);
        let h = &hits[0];
        assert_eq!(h.id, "AABBCCDD");
        assert_eq!(h.slug, "sodium");
        assert_eq!(h.title, "Sodium");
        assert_eq!(h.author, "jellysquid3");
        assert_eq!(h.downloads, 12345);
        assert_eq!(h.icon_url, None);
        assert_eq!(h.categories, vec!["optimization".to_string(), "fabric".to_string()]);
    }

    #[test]
    fn parse_versions_maps_files_and_deps() {
        let sample = r#"[
            {
                "id": "VERSION1",
                "name": "Sodium 0.5.3",
                "version_number": "mc1.20.1-0.5.3",
                "game_versions": ["1.20.1"],
                "loaders": ["fabric"],
                "files": [
                    {
                        "url": "https://cdn.modrinth.com/data/x/y.jar",
                        "filename": "sodium-fabric-0.5.3.jar",
                        "hashes": { "sha1": "deadbeef", "sha512": "longhash" },
                        "size": 998877,
                        "primary": true
                    },
                    {
                        "url": "https://cdn.modrinth.com/data/x/z.jar",
                        "filename": "sources.jar",
                        "hashes": {},
                        "primary": false
                    }
                ],
                "dependencies": [
                    { "project_id": "DEP1", "dependency_type": "required" },
                    { "version_id": "DEPV", "dependency_type": "optional" },
                    { "project_id": "DEP3" }
                ]
            }
        ]"#;
        let vers = ModrinthApi::parse_versions(sample.as_bytes()).unwrap();
        assert_eq!(vers.len(), 1);
        let v = &vers[0];
        assert_eq!(v.id, "VERSION1");
        assert_eq!(v.version_number, "mc1.20.1-0.5.3");
        assert_eq!(v.game_versions, vec!["1.20.1".to_string()]);
        assert_eq!(v.loaders, vec!["fabric".to_string()]);

        assert_eq!(v.files.len(), 2);
        let primary = v.primary_file().unwrap();
        assert_eq!(primary.filename, "sodium-fabric-0.5.3.jar");
        assert_eq!(primary.sha1.as_deref(), Some("deadbeef"));
        assert_eq!(primary.size, Some(998877));
        assert!(primary.primary);
        // 第二个文件 hashes 为空对象 → sha1 None, size 缺失 → None
        assert_eq!(v.files[1].sha1, None);
        assert_eq!(v.files[1].size, None);
        assert!(!v.files[1].primary);

        assert_eq!(v.dependencies.len(), 3);
        assert_eq!(v.dependencies[0].project_id.as_deref(), Some("DEP1"));
        assert_eq!(v.dependencies[0].dependency_type, "required");
        assert_eq!(v.dependencies[1].version_id.as_deref(), Some("DEPV"));
        assert_eq!(v.dependencies[1].dependency_type, "optional");
        // 缺 dependency_type → 默认 "required"
        assert_eq!(v.dependencies[2].dependency_type, "required");
    }

    #[test]
    fn parse_project_maps_id_field() {
        // /project 端点用 `id`(非 project_id),且不带 author。
        let sample = r#"{
            "id": "PROJ123",
            "slug": "fabric-api",
            "title": "Fabric API",
            "description": "Core library",
            "downloads": 50000000,
            "icon_url": "https://cdn.modrinth.com/icon.png",
            "categories": ["library", "fabric"]
        }"#;
        let hit = ModrinthApi::parse_project(sample.as_bytes()).unwrap();
        assert_eq!(hit.id, "PROJ123");
        assert_eq!(hit.slug, "fabric-api");
        assert_eq!(hit.title, "Fabric API");
        assert_eq!(hit.author, "");
        assert_eq!(hit.downloads, 50_000_000);
        assert_eq!(hit.icon_url.as_deref(), Some("https://cdn.modrinth.com/icon.png"));
    }

    #[test]
    fn primary_file_falls_back_to_first() {
        let v = ProjectVersion {
            id: "v".into(),
            name: "n".into(),
            version_number: "1".into(),
            game_versions: vec![],
            loaders: vec![],
            files: vec![
                VersionFile { url: "a".into(), filename: "a".into(), sha1: None, size: None, primary: false },
                VersionFile { url: "b".into(), filename: "b".into(), sha1: None, size: None, primary: false },
            ],
            dependencies: vec![],
        };
        assert_eq!(v.primary_file().unwrap().filename, "a");
    }

    #[test]
    fn empty_hits_default() {
        // 完全空对象也能解析(total_hits 缺失、hits 缺失 → 空 Vec)。
        let hits = ModrinthApi::parse_search_response(b"{}").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn malformed_json_yields_parse_error() {
        let err = ModrinthApi::parse_versions(b"not json").unwrap_err();
        assert!(matches!(err, CoreError::Parse { .. }));
    }
}
