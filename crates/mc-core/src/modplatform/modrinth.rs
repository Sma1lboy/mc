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

    /// 取**单个版本**的元信息(`GET /v2/version/{id}`)。导入时把 manifest 里的
    /// version id 变成可下载文件,逐个走这个端点。映射复用 [`map_version`]。
    pub async fn get_version(&self, version_id: &str) -> Result<ProjectVersion> {
        let url = format!("{}/version/{}", self.base, version_id);
        let raw: RawVersion = self
            .client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(map_version(raw))
    }

    /// 按文件哈希批量反查版本(`POST /v2/version_files`)。
    ///
    /// 请求体形如 `{"hashes":["<h1>","<h2>"],"algorithm":"sha512"}`,
    /// `algorithm` 取 `"sha1"` 或 `"sha512"`。响应是一个 **json 对象**,键为
    /// *请求时传入的哈希*、值为对应的版本对象(同 `/version/{id}` 形状)。未命中
    /// 的哈希直接从对象里缺席——因此返回的 [`HashMap`] 可能比输入短。
    pub async fn versions_from_hashes(
        &self,
        hashes: &[String],
        algorithm: &str,
    ) -> Result<std::collections::HashMap<String, ProjectVersion>> {
        let raw = self.raw_versions_from_hashes(hashes, algorithm).await?;
        Ok(raw.into_iter().map(|(k, v)| (k, map_version(v))).collect())
    }

    /// 批量取项目元信息(`GET /v2/projects?ids=["a","b"]`)。`ids` 参数是 json 编码
    /// 的字符串数组。响应是项目对象数组(同 `/project/{id}` 形状),逐个走 [`map_project`]。
    pub async fn get_projects(&self, ids: &[String]) -> Result<Vec<SearchHit>> {
        let url = format!("{}/projects", self.base);
        let id_refs: Vec<&str> = ids.iter().map(|s| s.as_str()).collect();
        let ids_param = json_string_array(&id_refs);
        let bytes = self
            .client
            .get(&url)
            .query(&[("ids", ids_param.as_str())])
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Self::parse_projects(&bytes)
    }

    /// 便捷方法:解析 `/version_files` 的响应对象(hash → 版本)字节。
    /// 失败映射成 [`CoreError::Parse`]。
    pub fn parse_versions_from_hashes(
        bytes: &[u8],
    ) -> Result<std::collections::HashMap<String, ProjectVersion>> {
        let raw = Self::parse_raw_versions_from_hashes(bytes)?;
        Ok(raw.into_iter().map(|(k, v)| (k, map_version(v))).collect())
    }

    /// 同 [`Self::parse_versions_from_hashes`],但保留 [`RawVersion`](含 `project_id`),
    /// 供哈希反查(`resolve_by_hashes`)构造 [`ResolvedFile`] 时取得项目 id。
    /// 仅模块内可见([`RawVersion`] 是私有承接类型,不外泄)。
    fn parse_raw_versions_from_hashes(
        bytes: &[u8],
    ) -> Result<std::collections::HashMap<String, RawVersion>> {
        serde_json::from_slice(bytes).map_err(|e| CoreError::Parse {
            what: "modrinth version_files response".into(),
            source: e,
        })
    }

    /// 同 [`Self::versions_from_hashes`],但返回保留 `project_id` 的原始版本对象。
    /// 哈希反查内部用——公开方法返回的统一 [`ProjectVersion`] 不带 project_id。
    async fn raw_versions_from_hashes(
        &self,
        hashes: &[String],
        algorithm: &str,
    ) -> Result<std::collections::HashMap<String, RawVersion>> {
        let url = format!("{}/version_files", self.base);
        let body = serde_json::json!({
            "hashes": hashes,
            "algorithm": algorithm,
        });
        let bytes = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Self::parse_raw_versions_from_hashes(&bytes)
    }

    /// 便捷方法:解析 `/projects` 的项目对象数组字节。失败映射成 [`CoreError::Parse`]。
    pub fn parse_projects(bytes: &[u8]) -> Result<Vec<SearchHit>> {
        let raws: Vec<RawProject> = serde_json::from_slice(bytes)
            .map_err(|e| CoreError::Parse { what: "modrinth projects".into(), source: e })?;
        Ok(raws.into_iter().map(map_project).collect())
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
    /// 该版本所属项目 id。`/version/{id}` 与 `/version_files` 的版本对象都带它,
    /// 用于哈希反查时构造 [`ResolvedFile::project_id`]。
    #[serde(default)]
    project_id: String,
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
    // —— 详情页额外字段(map_version 不消费,version_details 用)——
    #[serde(default)]
    version_type: String,
    #[serde(default)]
    date_published: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    changelog: String,
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
    #[serde(default)]
    sha512: Option<String>,
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

/// 一个版本的展示用详情(整合包详情页:含 changelog / 发布时间 / 类型 / 下载数,
/// 以及该版本的 `.mrpack` 文件地址)。比统一的 [`ProjectVersion`] 多带 UI 信息。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VersionDetail {
    pub id: String,
    pub version_number: String,
    pub name: String,
    /// `release` / `beta` / `alpha`。
    pub version_type: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    /// ISO 8601 发布时间。
    pub date_published: String,
    pub downloads: u64,
    /// 该版本的更新日志(markdown 原文)。
    pub changelog: String,
    /// 该版本的 `.mrpack` 下载地址(供「安装此版本」用);无则 `None`。
    pub mrpack_url: Option<String>,
    pub mrpack_filename: Option<String>,
    pub file_size: Option<u64>,
}

fn map_version_detail(r: RawVersion) -> VersionDetail {
    // 优先 .mrpack 文件,其次 primary,其次第一个。
    let file = r
        .files
        .iter()
        .find(|f| f.filename.to_ascii_lowercase().ends_with(".mrpack"))
        .or_else(|| r.files.iter().find(|f| f.primary))
        .or_else(|| r.files.first());
    let (mrpack_url, mrpack_filename, file_size) = match file {
        Some(f) => (Some(f.url.clone()), Some(f.filename.clone()), f.size),
        None => (None, None, None),
    };
    VersionDetail {
        id: r.id,
        version_number: r.version_number,
        name: r.name,
        version_type: r.version_type,
        game_versions: r.game_versions,
        loaders: r.loaders,
        date_published: r.date_published,
        downloads: r.downloads,
        changelog: r.changelog,
        mrpack_url,
        mrpack_filename,
        file_size,
    }
}

impl ModrinthApi {
    /// 列出某项目所有版本的展示详情(含 changelog / 类型 / 发布时间 + `.mrpack` 地址)。
    /// 整合包详情页用。
    pub async fn version_details(&self, project_id: &str) -> Result<Vec<VersionDetail>> {
        let url = format!("{}/project/{}/version", self.base, project_id);
        let raws: Vec<RawVersion> =
            self.client.get(&url).send().await?.error_for_status()?.json().await?;
        Ok(raws.into_iter().map(map_version_detail).collect())
    }
}

fn map_file(r: RawFile) -> VersionFile {
    VersionFile {
        url: r.url,
        filename: r.filename,
        sha1: r.hashes.sha1,
        sha512: r.hashes.sha512,
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

// ============================ ResourceProvider 适配 ============================

use std::collections::HashMap;

use futures::future::{try_join_all, BoxFuture};

use super::provider::ResourceProvider;
use super::{HashAlgo, ProviderCaps, ProviderId, ResolvedFile, SearchQuery};

/// Modrinth 支持反查的哈希算法,按偏好序(sha512 优先,sha1 兜底)。
/// `&'static [HashAlgo]` 需要一个 `'static` 数组,故声明为 const。
const MODRINTH_HASH_ALGOS: &[HashAlgo] = &[HashAlgo::Sha512, HashAlgo::Sha1];

/// Modrinth 的能力声明(`const`,无运行时输入)。
const MODRINTH_CAPS: ProviderCaps = ProviderCaps {
    id: ProviderId::Modrinth,
    readable_name: "Modrinth",
    hash_algos: MODRINTH_HASH_ALGOS,
    needs_api_key: false,
};

/// 把统一 [`SearchQuery`] 适配到 [`ModrinthApi`] 的 [`ResourceProvider`] 实现。
/// 持有一个 [`ModrinthApi`](内含配好 UA 的 `reqwest::Client`)。
#[derive(Debug, Clone, Default)]
pub struct ModrinthProvider {
    api: ModrinthApi,
}

impl ModrinthProvider {
    /// 默认 base url(`https://api.modrinth.com/v2`)的 provider。
    pub fn new() -> Self {
        Self { api: ModrinthApi::new() }
    }

    /// 用自定义 base url 构造(测试 / 镜像)。
    pub fn with_base(base: impl Into<String>) -> Self {
        Self { api: ModrinthApi::with_base(base) }
    }
}

/// 把统一 [`HashAlgo`] 映射到 Modrinth `/version_files` 的 `algorithm` 字符串。
/// Modrinth 只支持 sha1 / sha512;其余算法不可反查。
fn modrinth_algo_str(algo: HashAlgo) -> Result<&'static str> {
    match algo {
        HashAlgo::Sha512 => Ok("sha512"),
        HashAlgo::Sha1 => Ok("sha1"),
        HashAlgo::Md5 | HashAlgo::Murmur2 => {
            Err(CoreError::other("unsupported hash algo for Modrinth"))
        }
    }
}

/// 在一个版本的文件里找出哈希(sha1/sha512)等于 `wanted` 的那一个。
/// 一个版本可能有多文件(主 jar、sources 等),反查命中的是某一个具体文件,
/// 必须按算法逐个比对哈希,而不能假定是主文件。
fn find_file_by_hash(version: &RawVersion, algo: HashAlgo, wanted: &str) -> Option<VersionFile> {
    version.files.iter().find_map(|f| {
        let h = match algo {
            HashAlgo::Sha512 => f.hashes.sha512.as_deref(),
            HashAlgo::Sha1 => f.hashes.sha1.as_deref(),
            _ => None,
        }?;
        // Modrinth 哈希是十六进制小写;大小写无关地比一次更稳。
        if h.eq_ignore_ascii_case(wanted) {
            Some(map_file_ref(f))
        } else {
            None
        }
    })
}

/// 借引用把 [`RawFile`] 映射到 [`VersionFile`](`map_file` 是消费式,这里给反查用)。
fn map_file_ref(r: &RawFile) -> VersionFile {
    VersionFile {
        url: r.url.clone(),
        filename: r.filename.clone(),
        sha1: r.hashes.sha1.clone(),
        sha512: r.hashes.sha512.clone(),
        size: r.size,
        primary: r.primary,
    }
}

impl ResourceProvider for ModrinthProvider {
    fn caps(&self) -> &ProviderCaps {
        &MODRINTH_CAPS
    }

    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move {
            self.api
                .search(
                    &q.text,
                    q.kind,
                    q.game_version.as_deref(),
                    q.loader.as_deref(),
                    q.limit,
                )
                .await
        })
    }

    fn get_project<'a>(&'a self, project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
        Box::pin(async move { self.api.get_project(project_id).await })
    }

    fn get_projects<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move { self.api.get_projects(project_ids).await })
    }

    fn list_versions<'a>(
        &'a self,
        project_id: &'a str,
        game_version: Option<&'a str>,
        loader: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
        Box::pin(async move { self.api.get_versions(project_id, game_version, loader).await })
    }

    fn resolve_by_hashes<'a>(
        &'a self,
        algo: HashAlgo,
        hashes: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
        Box::pin(async move {
            let algorithm = modrinth_algo_str(algo)?;
            let by_hash: HashMap<String, RawVersion> =
                self.api.raw_versions_from_hashes(hashes, algorithm).await?;

            // 输出严格与输入 `hashes` 对齐:逐个查表,命中后再在版本的文件里按算法
            // 找到哈希恰好相等的那个文件(一个版本可能挂多个文件)。
            let out = hashes
                .iter()
                .map(|h| {
                    let version = by_hash.get(h)?;
                    let file = find_file_by_hash(version, algo, h)?;
                    Some(ResolvedFile {
                        provider: ProviderId::Modrinth,
                        project_id: version.project_id.clone(),
                        version_id: version.id.clone(),
                        file,
                        project_name: None,
                        project_slug: None,
                        authors: Vec::new(),
                    })
                })
                .collect();
            Ok(out)
        })
    }

    fn get_files_bulk<'a>(
        &'a self,
        refs: &'a [(String, String)],
    ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
        Box::pin(async move {
            // Modrinth 无批量 version 端点,逐个 `/version/{id}` 并发取。`refs` 是
            // (project_id, version_id);项目 id 直接用作 ResolvedFile.project_id。
            let futures = refs.iter().map(|(project_id, version_id)| async move {
                let version = self.api.get_version(version_id).await?;
                // 主文件即下载目标;没有文件的版本视为无法解析。
                let file = version.primary_file().cloned().ok_or_else(|| {
                    CoreError::other(format!("Modrinth version {version_id} has no files"))
                })?;
                Ok::<ResolvedFile, CoreError>(ResolvedFile {
                    provider: ProviderId::Modrinth,
                    project_id: project_id.clone(),
                    version_id: version.id.clone(),
                    file,
                    project_name: None,
                    project_slug: None,
                    authors: Vec::new(),
                })
            });
            try_join_all(futures).await
        })
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
                VersionFile { url: "a".into(), filename: "a".into(), primary: false, ..Default::default() },
                VersionFile { url: "b".into(), filename: "b".into(), primary: false, ..Default::default() },
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

    // -------------------- /version_files (hash → version) --------------------

    #[test]
    fn parse_versions_from_hashes_maps_object_keyed_by_hash() {
        // 响应是对象:键=请求时传入的哈希,值=版本对象。覆盖 project_id 字段、
        // 缺席哈希(只回了一个键)、以及多文件版本。
        let sample = r#"{
            "abc123sha512": {
                "id": "VER_A",
                "project_id": "PROJ_A",
                "name": "Sodium 0.5.3",
                "version_number": "0.5.3",
                "game_versions": ["1.20.1"],
                "loaders": ["fabric"],
                "files": [
                    {
                        "url": "https://cdn.modrinth.com/data/a/sodium.jar",
                        "filename": "sodium.jar",
                        "hashes": { "sha1": "aaa", "sha512": "abc123sha512" },
                        "size": 100,
                        "primary": true
                    }
                ],
                "dependencies": []
            }
        }"#;
        let map = ModrinthApi::parse_versions_from_hashes(sample.as_bytes()).unwrap();
        assert_eq!(map.len(), 1);
        let v = map.get("abc123sha512").expect("hash key present");
        assert_eq!(v.id, "VER_A");
        assert_eq!(v.version_number, "0.5.3");
        assert_eq!(v.files.len(), 1);
        assert_eq!(v.files[0].sha512.as_deref(), Some("abc123sha512"));
        // 请求里多传一个未命中的哈希时,它就是不在 map 里——这里模拟"只回一个键"。
        assert!(map.get("missinghash").is_none());
    }

    #[test]
    fn parse_versions_from_hashes_empty_object() {
        // 全部未命中 → 空对象 → 空 map。
        let map = ModrinthApi::parse_versions_from_hashes(b"{}").unwrap();
        assert!(map.is_empty());
    }

    #[test]
    fn parse_versions_from_hashes_malformed_is_parse_error() {
        let err = ModrinthApi::parse_versions_from_hashes(b"[not an object]").unwrap_err();
        assert!(matches!(err, CoreError::Parse { .. }));
    }

    #[test]
    fn raw_versions_from_hashes_keeps_project_id() {
        // 内部 raw 解析必须保留 project_id(统一 ProjectVersion 不带它)。
        let sample = r#"{
            "h1": {
                "id": "VER_X",
                "project_id": "PROJ_X",
                "files": [
                    { "url": "u", "filename": "f.jar", "hashes": { "sha1": "h1" }, "primary": true }
                ]
            }
        }"#;
        let raw = ModrinthApi::parse_raw_versions_from_hashes(sample.as_bytes()).unwrap();
        let v = raw.get("h1").unwrap();
        assert_eq!(v.project_id, "PROJ_X");
        assert_eq!(v.id, "VER_X");
    }

    // ------------------------------ /projects --------------------------------

    #[test]
    fn parse_projects_maps_array_of_projects() {
        // 数组形状,字段同 /project/{id}(id 字段叫 `id`,无 author)。
        let sample = r#"[
            {
                "id": "PROJ1",
                "slug": "fabric-api",
                "title": "Fabric API",
                "description": "Core library",
                "downloads": 50000000,
                "icon_url": "https://cdn.modrinth.com/icon.png",
                "categories": ["library", "fabric"]
            },
            {
                "id": "PROJ2",
                "slug": "sodium",
                "title": "Sodium",
                "description": "Rendering engine",
                "downloads": 12345,
                "categories": ["optimization"]
            }
        ]"#;
        let hits = ModrinthApi::parse_projects(sample.as_bytes()).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "PROJ1");
        assert_eq!(hits[0].slug, "fabric-api");
        assert_eq!(hits[0].author, ""); // /projects 端点不带 author
        assert_eq!(hits[0].downloads, 50_000_000);
        assert_eq!(hits[1].id, "PROJ2");
        assert_eq!(hits[1].title, "Sodium");
    }

    #[test]
    fn parse_projects_empty_array() {
        let hits = ModrinthApi::parse_projects(b"[]").unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn parse_projects_malformed_is_parse_error() {
        let err = ModrinthApi::parse_projects(b"{}").unwrap_err();
        assert!(matches!(err, CoreError::Parse { .. }));
    }

    // ------------------- provider: caps / algo / hash match -------------------

    #[test]
    fn provider_caps_are_modrinth() {
        let p = ModrinthProvider::new();
        let caps = p.caps();
        assert_eq!(caps.id, ProviderId::Modrinth);
        assert_eq!(caps.readable_name, "Modrinth");
        assert!(!caps.needs_api_key);
        assert_eq!(caps.hash_algos, &[HashAlgo::Sha512, HashAlgo::Sha1]);
        assert_eq!(p.id(), ProviderId::Modrinth);
    }

    #[test]
    fn algo_str_maps_supported_and_rejects_others() {
        assert_eq!(modrinth_algo_str(HashAlgo::Sha512).unwrap(), "sha512");
        assert_eq!(modrinth_algo_str(HashAlgo::Sha1).unwrap(), "sha1");
        assert!(matches!(
            modrinth_algo_str(HashAlgo::Md5),
            Err(CoreError::Other(_))
        ));
        assert!(matches!(
            modrinth_algo_str(HashAlgo::Murmur2),
            Err(CoreError::Other(_))
        ));
    }

    #[test]
    fn find_file_by_hash_picks_the_matching_file_not_the_primary() {
        // 一个版本两文件:primary 是主 jar(sha512=primaryhash),另一个 sources
        // (sha512=wanted)。按哈希反查时应命中 sources,而非主文件。
        let sample = r#"{
            "id": "VER",
            "project_id": "PROJ",
            "files": [
                {
                    "url": "https://cdn/main.jar",
                    "filename": "main.jar",
                    "hashes": { "sha1": "p1", "sha512": "PRIMARYHASH" },
                    "primary": true
                },
                {
                    "url": "https://cdn/sources.jar",
                    "filename": "sources.jar",
                    "hashes": { "sha1": "s1", "sha512": "WANTEDHASH" },
                    "primary": false
                }
            ]
        }"#;
        let v: RawVersion = serde_json::from_str(sample).unwrap();

        let matched = find_file_by_hash(&v, HashAlgo::Sha512, "WANTEDHASH").unwrap();
        assert_eq!(matched.filename, "sources.jar");
        assert!(!matched.primary);

        // 大小写无关比对。
        let matched_ci = find_file_by_hash(&v, HashAlgo::Sha512, "wantedhash").unwrap();
        assert_eq!(matched_ci.filename, "sources.jar");

        // sha1 维度命中主文件。
        let by_sha1 = find_file_by_hash(&v, HashAlgo::Sha1, "p1").unwrap();
        assert_eq!(by_sha1.filename, "main.jar");

        // 不存在的哈希 → None。
        assert!(find_file_by_hash(&v, HashAlgo::Sha512, "nope").is_none());
    }

    #[test]
    fn resolve_alignment_pure_logic() {
        // 不打网络:直接验证"输出与输入 hashes 严格对齐、未命中为 None"的纯逻辑,
        // 复用 resolve_by_hashes 内部用到的同一组函数(parse + find_file_by_hash)。
        let sample = r#"{
            "HASH_A": {
                "id": "VER_A",
                "project_id": "PROJ_A",
                "files": [
                    { "url": "uA", "filename": "a.jar", "hashes": { "sha512": "HASH_A" }, "primary": true }
                ]
            }
        }"#;
        let by_hash = ModrinthApi::parse_raw_versions_from_hashes(sample.as_bytes()).unwrap();

        let inputs = vec!["HASH_A".to_string(), "HASH_MISSING".to_string()];
        let out: Vec<Option<ResolvedFile>> = inputs
            .iter()
            .map(|h| {
                let version = by_hash.get(h)?;
                let file = find_file_by_hash(version, HashAlgo::Sha512, h)?;
                Some(ResolvedFile {
                    provider: ProviderId::Modrinth,
                    project_id: version.project_id.clone(),
                    version_id: version.id.clone(),
                    file,
                    project_name: None,
                    project_slug: None,
                    authors: Vec::new(),
                })
            })
            .collect();

        assert_eq!(out.len(), 2);
        let r0 = out[0].as_ref().expect("HASH_A resolves");
        assert_eq!(r0.provider, ProviderId::Modrinth);
        assert_eq!(r0.project_id, "PROJ_A");
        assert_eq!(r0.version_id, "VER_A");
        assert_eq!(r0.file.filename, "a.jar");
        assert!(out[1].is_none()); // 未命中保持 None,下标对齐
    }
}
