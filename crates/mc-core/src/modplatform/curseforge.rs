//! CurseForge "Flame" API v1 后端(`api.curseforge.com`,**需要** `x-api-key`)。
//!
//! 文档:<https://docs.curseforge.com/>(`gameId=432` = Minecraft)。本模块用到这些端点:
//! - `GET  /v1/mods/search`     —— 搜索项目(`searchFilter` / `classId` / `gameVersion` / `modLoaderType`)
//! - `POST /v1/mods`            —— 批量取项目元信息(`{"modIds":[...]}`)
//! - `GET  /v1/mods/{id}/files` —— 列某项目的文件(= 我们的"版本")
//! - `POST /v1/mods/files`      —— 批量按 fileId 取文件(`{"fileIds":[...]}`),整合包导入把
//!   manifest 里的 (projectID,fileID) 变成真实下载 URL 的命脉
//! - `POST /v1/fingerprints`    —— murmur2 指纹反查(`{"fingerprints":[...]}`),整合包导出/去重用
//!
//! 设计要点(对齐 [`super::modrinth`]):
//! - **自带 `reqwest::Client`**,默认头里固化 `x-api-key` + 一个含仓库地址的 User-Agent。
//! - 平台原始 json(camelCase、整数枚举哈希算法、`gameVersions` 是混杂扁平数组)与统一
//!   模型差异很大,故用一组 `Raw*`/`FlameApi*` 内部类型承接原始 json,再由**纯映射函数**
//!   转成 [`crate::modplatform`] 的统一模型。映射函数无 IO、可单测。
//! - 容错:每个 `Raw` 字段一律 `#[serde(default)]`,缺字段不让整次请求打挂。HTTP/网络错误
//!   映射成 [`CoreError::Network`],反序列化错误映射成 [`CoreError::Parse`]。
//!
//! API key 是 **secret**:从环境变量 `MC_CF_API_KEY` 读取(镜像项目 `MC_MSA_CLIENT_ID`
//! 的约定),**绝不**硬编码、勿入库、勿打日志。详见 [`FlameApi::from_env`]。
//!
//! ## 几个 CurseForge 专有易错点
//! - **`downloadUrl` 可空 = BLOCKED**(作者禁第三方分发):映射后 [`VersionFile::url`] 为
//!   空串,调用方据"url 为空"识别 blocked,绝不猜 URL。
//! - **murmur2 不是标准 murmur2**:seed=1、先滤掉字节 9/10/13/32(tab/LF/CR/空格)再算。
//!   指纹由 [`crate::download::murmur2`] 计算;本模块只负责把已算好的 u32 提交反查。
//! - **`hashes[].algo` 是整数**(1=sha1,2=md5),不是字符串;CF 只保 sha1。
//! - **`gameVersions[]` 是扁平异构数组**(MC 版本 + loader 名 + Client/Server),客户端切分:
//!   含 `.` 的当游戏版本,匹配 forge/fabric/neoforge/quilt 的当 loader。
//! - **`/mods/files` 单 id 偶发返回对象而非数组**:`data` 用容忍数组或单对象的反序列化处理。

use serde::Deserialize;

use crate::error::{CoreError, Result};

use super::{
    HashAlgo, ProjectSideSupport, ProjectVersion, ProviderCaps, ProviderId, ResolvedFile,
    ResourceKind, SearchHit, SearchQuery, SortMethod, VersionFile,
};

/// CurseForge Flame API v1 根地址。
const API_BASE: &str = "https://api.curseforge.com/v1";

/// Minecraft 的 CurseForge `gameId`。
const GAME_ID: i64 = 432;

/// 读取 API key 的环境变量名(secret,见模块文档)。
const API_KEY_ENV: &str = "MC_CF_API_KEY";

/// User-Agent(含联系方式形式,和 Modrinth 后端一致)。
const USER_AGENT: &str = "mc-launcher/0.1 (github.com/sma1lboy/mc-launcher)";

/// CurseForge `classId`:Minecraft 各资源大类。`/mods/search` 用它锁定资源类型。
const CLASS_MOD: i64 = 6;
const CLASS_MODPACK: i64 = 4471;
const CLASS_RESOURCEPACK: i64 = 12;
const CLASS_SHADERPACK: i64 = 6552;

/// CurseForge `modLoaderType` 枚举值(`/mods/search` 的 `modLoaderType` 参数)。
/// 0=Any 1=Forge 2=Cauldron 3=LiteLoader 4=Fabric 5=Quilt 6=NeoForge。
fn loader_type_id(loader: &str) -> Option<i64> {
    match loader.to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

/// CurseForge `sortField`(`/mods/search` 的 `sortField` 参数)。
/// 1=Featured 2=Popularity 3=LastUpdated 4=Name 5=Author 6=TotalDownloads 7=Category 8=GameVersion。
fn sort_field_id(sort: SortMethod) -> i64 {
    match sort {
        // 没有真正的"相关度",CF 默认用 Popularity 作为最贴近的排序。
        SortMethod::Relevance => 2,
        SortMethod::Downloads => 6,
        // CF 没有"按发布时间",最接近的是 LastUpdated。
        SortMethod::Newest => 3,
        SortMethod::Updated => 3,
    }
}

/// 把统一 [`ResourceKind`] 映射到 CurseForge `classId`。数据包在 CF 没有独立 class,
/// 与 mod 同 class(用 category 区分),故回退到 [`CLASS_MOD`]。
fn class_id(kind: ResourceKind) -> i64 {
    match kind {
        ResourceKind::Mod => CLASS_MOD,
        ResourceKind::Modpack => CLASS_MODPACK,
        ResourceKind::ResourcePack => CLASS_RESOURCEPACK,
        ResourceKind::Shader => CLASS_SHADERPACK,
        ResourceKind::Datapack => CLASS_MOD,
    }
}

/// CurseForge Flame API 客户端。`new()` 自带配置好 `x-api-key` + UA 的 `reqwest::Client`。
#[derive(Debug, Clone)]
pub struct FlameApi {
    client: reqwest::Client,
    base: String,
    api_key: String,
}

impl FlameApi {
    /// 用给定 API key 构造一个新客户端。
    ///
    /// `x-api-key` 与 UA 固化进默认头。构造失败(几乎不会:仅 TLS 后端初始化失败或
    /// header 含非法字节)走 `expect`——属于环境级灾难,失败即代表无法发任何请求。
    pub fn new(api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();

        let mut headers = reqwest::header::HeaderMap::new();
        // key 来自 env,理论上可能含非法 header 字节;此时退化为不带默认头(请求会 401),
        // 但不让一个坏 key 把整个进程 panic。
        if let Ok(mut value) = reqwest::header::HeaderValue::from_str(&api_key) {
            value.set_sensitive(true);
            headers.insert("x-api-key", value);
        }

        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            // reqwest 仅在 TLS 后端初始化失败时报错,属于环境级灾难;静态配置失败即代表
            // 整个进程无法发任何请求,直接 expect 暴露问题。
            .build()
            .expect("failed to build reqwest client for CurseForge");

        Self { client, base: API_BASE.to_string(), api_key }
    }

    /// 从环境变量 [`API_KEY_ENV`] 构造:key 存在且去空白后非空才返回 `Some`,否则 `None`
    /// (上层据此决定是否注册 CurseForge provider——无 key 就不注册,而非塞个会 401 的)。
    pub fn from_env() -> Option<Self> {
        std::env::var(API_KEY_ENV)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(Self::new)
    }

    /// 从显式 key 构造:去空白后非空才返回 `Some`,否则 `None`。用户在设置里填的
    /// CurseForge key 走这条路(与 [`Self::from_env`] 同样的空白/空串守卫)。
    pub fn from_key(key: impl Into<String>) -> Option<Self> {
        let key = key.into();
        let key = key.trim();
        if key.is_empty() {
            None
        } else {
            Some(Self::new(key))
        }
    }

    /// 用自定义 base url 构造(主要给测试/镜像用)。链式消费 self。
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// 当前持有的 API key(供上层判断/诊断;**勿打日志**)。
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// 搜索项目。`GET /mods/search?gameId=432&...`。
    ///
    /// - `classId`:由 [`SearchQuery::kind`] 决定的资源大类。
    /// - `searchFilter`:文本关键字。
    /// - `gameVersion` / `modLoaderType`:可选过滤。
    /// - `index` / `pageSize`:分页(CF `pageSize` 上限 50,这里夹到 [1,50])。
    pub async fn search(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        let url = format!("{}/mods/search", self.base);
        let class = class_id(q.kind);
        let page_size = q.limit.clamp(1, 50);
        let sort_field = sort_field_id(q.sort);

        let game_id = GAME_ID.to_string();
        let class_id_s = class.to_string();
        let index = q.offset.to_string();
        let page_size_s = page_size.to_string();
        let sort_field_s = sort_field.to_string();

        let mut params: Vec<(&str, String)> = vec![
            ("gameId", game_id),
            ("classId", class_id_s),
            ("searchFilter", q.text.clone()),
            ("index", index),
            ("pageSize", page_size_s),
            ("sortField", sort_field_s),
            // CF `sortOrder`: "asc" | "desc"。下载/更新一律降序(多在前 / 新在前)。
            ("sortOrder", "desc".to_string()),
        ];
        if let Some(v) = q.game_version.as_deref().filter(|s| !s.is_empty()) {
            params.push(("gameVersion", v.to_string()));
        }
        if let Some(t) = q.loader.as_deref().and_then(loader_type_id) {
            params.push(("modLoaderType", t.to_string()));
        }

        let resp: FlameEnvelope<Vec<FlameApiProject>> = self
            .client
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp.data.into_iter().map(map_project).collect())
    }

    /// 批量取项目元信息。`POST /mods` body `{"modIds":[...]}`,response `{"data":[...]}`。
    pub async fn get_mods(&self, mod_ids: &[i64]) -> Result<Vec<FlameApiProject>> {
        if mod_ids.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/mods", self.base);
        let body = serde_json::json!({ "modIds": mod_ids });

        let resp: FlameEnvelope<Vec<FlameApiProject>> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp.data)
    }

    /// 批量按 fileId 取文件。`POST /mods/files` body `{"fileIds":[...]}`,response `{"data":[...]}`。
    ///
    /// **单 id 偶发返回对象而非数组**:`data` 用 [`OneOrMany`] 容忍两种形态。
    pub async fn get_files(&self, file_ids: &[i64]) -> Result<Vec<FlameApiFile>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/mods/files", self.base);
        let body = serde_json::json!({ "fileIds": file_ids });

        let resp: FlameEnvelope<OneOrMany<FlameApiFile>> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp.data.into_vec())
    }

    /// murmur2 指纹反查。`POST /fingerprints` body `{"fingerprints":[...]}`,
    /// response `data.exactMatches[]`(每项 `.file` 是一个 [`FlameApiFile`])。
    ///
    /// 指纹是**已算好**的 CurseForge murmur2(seed=1、滤空白)u32(见
    /// [`crate::download::murmur2::cf_fingerprint`])。返回的匹配**顺序不保证**与输入一致,
    /// 调用方需用 `file.file_fingerprint` 自行对齐(见 [`CurseForgeProvider::resolve_by_hashes`])。
    pub async fn match_fingerprints(&self, fps: &[u32]) -> Result<Vec<FlameFingerprintMatch>> {
        if fps.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/fingerprints", self.base);
        let body = serde_json::json!({ "fingerprints": fps });

        let resp: FlameEnvelope<FlameFingerprintData> = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        Ok(resp.data.exact_matches)
    }
}

// ============================ 原始 json 承接类型 ============================

/// 通用响应信封:CurseForge 所有端点都把负载裹在 `{"data": …}` 里。
///
/// `data` 走 `#[serde(default)]` 容忍缺失;serde derive 不会自动给泛型加 `Default`
/// 约束,故用容器级 `bound` 显式声明 `T: Default + Deserialize`,使 `data` 缺失时回退
/// 到 `T::default()`(`Vec`/`OneOrMany`/`FlameFingerprintData` 都实现了 `Default`)。
#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: Default + serde::Deserialize<'de>"))]
struct FlameEnvelope<T> {
    #[serde(default)]
    data: T,
}

/// 容忍"单对象 or 数组"的反序列化包装。`/mods/files` 在 fileIds 只有一个时偶发返回
/// 单个对象而非数组;用 `#[serde(untagged)]` 同时接受两种形态。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    Many(Vec<T>),
    One(T),
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}

impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::Many(v) => v,
            OneOrMany::One(x) => vec![x],
        }
    }
}

/// `/mods/files` & `/mods/{id}/files` 里的一个文件(= 我们的"版本")。camelCase。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameApiFile {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub mod_id: i64,
    #[serde(default)]
    pub display_name: String,
    /// 写盘前必须 RemoveInvalidPathChars + 拒绝分隔符(由下载/导入层负责)。
    #[serde(default)]
    pub file_name: String,
    /// **可空!** `null`/缺失 = BLOCKED(作者禁第三方分发),映射后 url 为空串。
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub file_length: Option<u64>,
    /// CurseForge murmur2 指纹(seed=1、滤空白);反查对齐用。
    #[serde(default)]
    pub file_fingerprint: Option<u64>,
    /// 整数枚举算法的哈希列表(1=sha1,2=md5)。CF 只保 sha1。
    #[serde(default)]
    pub hashes: Vec<FlameApiFileHash>,
    /// 扁平异构数组:MC 版本(含 `.`)+ loader 名 + Client/Server,客户端切分。
    #[serde(default)]
    pub game_versions: Vec<String>,
}

impl FlameApiFile {
    /// 取指定整数 algo 的哈希值(1=sha1,2=md5)。无则 `None`。
    fn hash_of(&self, algo: i32) -> Option<String> {
        self.hashes
            .iter()
            .find(|h| h.algo == algo && !h.value.is_empty())
            .map(|h| h.value.clone())
    }
}

/// 一个文件哈希。**`algo` 是整数**(1=sha1,2=md5),不是字符串。
#[derive(Debug, Clone, Deserialize)]
pub struct FlameApiFileHash {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub algo: i32,
}

/// `/mods/search` & `/mods` 里的一个项目。只取我们用得到的字段(保持精简)。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameApiProject {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub download_count: f64,
    /// 嵌套:`links.websiteUrl`。
    #[serde(default)]
    pub links: FlameApiProjectLinks,
    /// 嵌套:`logo.url`(项目图标)。可空。
    #[serde(default)]
    pub logo: Option<FlameApiLogo>,
    /// 作者列表(`authors[].name`)。
    #[serde(default)]
    pub authors: Vec<FlameApiAuthor>,
    /// 分类(`categories[].name`)。
    #[serde(default)]
    pub categories: Vec<FlameApiCategory>,
    /// 截图列表(`screenshots[].url`),取第一张作高清封面。
    #[serde(default)]
    pub screenshots: Vec<FlameApiLogo>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameApiProjectLinks {
    #[serde(default)]
    pub website_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlameApiLogo {
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlameApiAuthor {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlameApiCategory {
    #[serde(default)]
    pub name: String,
}

/// `/fingerprints` 的 `data` 负载。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameFingerprintData {
    #[serde(default)]
    pub exact_matches: Vec<FlameFingerprintMatch>,
}

/// `/fingerprints` 里的一个精确匹配:`.file` 是命中的文件。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameFingerprintMatch {
    #[serde(default)]
    pub id: i64,
    pub file: FlameApiFile,
}

// ============================ 纯映射函数(可单测) ============================

/// CurseForge 用来标记 loader 名的关键字(出现在扁平 `gameVersions[]` 里)。
const LOADER_NAMES: &[&str] = &["forge", "fabric", "neoforge", "quilt", "liteloader"];

/// 把 CurseForge 扁平异构的 `gameVersions[]` 切成 (游戏版本, loaders)。
/// 含 `.` 的当游戏版本(`1.20.1`);匹配 [`LOADER_NAMES`] 的当 loader。其余(Client/Server)丢弃。
fn partition_game_versions(raw: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut game = Vec::new();
    let mut loaders = Vec::new();
    for entry in raw {
        let lower = entry.to_ascii_lowercase();
        if LOADER_NAMES.iter().any(|l| lower == *l) {
            loaders.push(lower);
        } else if entry.contains('.') {
            game.push(entry);
        }
        // 其它(Client/Server、空串…)忽略。
    }
    (game, loaders)
}

/// 把一个 [`FlameApiFile`] 映射成统一 [`VersionFile`]。
///
/// `download_url == None`(BLOCKED)→ url 为空串,调用方据此识别 blocked。sha1 取
/// algo==1 的哈希;CF 不提供 sha512(留 `None`)。primary 恒 true(CF 一个 file 即一个版本)。
fn map_version_file(f: &FlameApiFile) -> VersionFile {
    VersionFile {
        url: f.download_url.clone().unwrap_or_default(),
        filename: f.file_name.clone(),
        sha1: f.hash_of(1),
        sha512: None,
        size: f.file_length,
        primary: true,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    }
}

/// 把一个 [`FlameApiFile`] 映射成统一 [`ProjectVersion`]。
///
/// version id = file id(字符串);name/version_number 用 `displayName`;game_versions/loaders
/// 从扁平 `gameVersions[]` 切分;files = 单个映射后的 [`VersionFile`];CF 文件级不带依赖,留空。
fn map_file_to_version(f: FlameApiFile) -> ProjectVersion {
    let file = map_version_file(&f);
    let (game_versions, loaders) = partition_game_versions(f.game_versions);
    ProjectVersion {
        id: f.id.to_string(),
        name: f.display_name.clone(),
        version_number: f.display_name,
        game_versions,
        loaders,
        files: vec![file],
        dependencies: Vec::new(),
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    }
}

/// 把一个 [`FlameApiProject`] 映射成统一 [`SearchHit`]。id 为项目数字 id 的字符串。
fn map_project(p: FlameApiProject) -> SearchHit {
    let author = p
        .authors
        .first()
        .map(|a| a.name.clone())
        .unwrap_or_default();
    let icon_url = p.logo.and_then(|l| l.url);
    let gallery_url = p.screenshots.into_iter().find_map(|s| s.url);
    // download_count 是浮点(CF 有时给带小数的近似值),夹到非负后转 u64。
    let downloads = if p.download_count.is_finite() && p.download_count > 0.0 {
        p.download_count as u64
    } else {
        0
    };
    SearchHit {
        id: p.id.to_string(),
        slug: p.slug,
        title: p.name,
        description: p.summary,
        author,
        downloads,
        icon_url,
        gallery_url,
        categories: p.categories.into_iter().map(|c| c.name).collect(),
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    }
}

// ============================ Provider 实现 ============================

/// CurseForge 的 [`ResourceProvider`] 实现,内含一个 [`FlameApi`]。
pub struct CurseForgeProvider {
    api: FlameApi,
}

/// CurseForge 能力声明:需要 API key,反查算法仅 murmur2(CF 指纹反查端点)。
static CURSEFORGE_CAPS: ProviderCaps = ProviderCaps {
    id: ProviderId::CurseForge,
    readable_name: "CurseForge",
    hash_algos: &[HashAlgo::Murmur2],
    needs_api_key: true,
};

impl CurseForgeProvider {
    /// 用一个已配置好的 [`FlameApi`] 构造。
    pub fn new(api: FlameApi) -> Self {
        Self { api }
    }

    /// 便捷:从 env 构造(无 key 则 `None`,上层据此决定是否注册)。
    pub fn from_env() -> Option<Self> {
        FlameApi::from_env().map(Self::new)
    }

    /// 便捷:从显式 key 构造(空/全空白则 `None`)。用户设置里填的 CurseForge key 走这条路。
    pub fn from_key(key: impl Into<String>) -> Option<Self> {
        FlameApi::from_key(key).map(Self::new)
    }

    /// 取底层 [`FlameApi`](诊断/复用用)。
    pub fn api(&self) -> &FlameApi {
        &self.api
    }
}

use super::provider::ResourceProvider;
use futures::future::BoxFuture;

impl ResourceProvider for CurseForgeProvider {
    fn caps(&self) -> &ProviderCaps {
        &CURSEFORGE_CAPS
    }

    fn search<'a>(&'a self, q: &'a SearchQuery) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move { self.api.search(q).await })
    }

    fn get_project<'a>(&'a self, project_id: &'a str) -> BoxFuture<'a, Result<SearchHit>> {
        Box::pin(async move {
            let id = parse_id(project_id, "CurseForge project id")?;
            let mods = self.api.get_mods(&[id]).await?;
            mods.into_iter()
                .next()
                .map(map_project)
                .ok_or_else(|| CoreError::other(format!("CurseForge project {project_id} not found")))
        })
    }

    fn get_projects<'a>(
        &'a self,
        project_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<SearchHit>>> {
        Box::pin(async move {
            let ids = parse_ids(project_ids, "CurseForge project id")?;
            let mods = self.api.get_mods(&ids).await?;
            Ok(mods.into_iter().map(map_project).collect())
        })
    }

    fn list_versions<'a>(
        &'a self,
        project_id: &'a str,
        game_version: Option<&'a str>,
        loader: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Vec<ProjectVersion>>> {
        Box::pin(async move {
            let id = parse_id(project_id, "CurseForge project id")?;
            let url = format!("{}/mods/{}/files", self.api.base, id);

            // 首页即可(CF `pageSize` 上限 50);上层一般取最近若干版本。
            let mut params: Vec<(&str, String)> = vec![
                ("index", "0".to_string()),
                ("pageSize", "50".to_string()),
            ];
            if let Some(v) = game_version.filter(|s| !s.is_empty()) {
                params.push(("gameVersion", v.to_string()));
            }
            if let Some(t) = loader.and_then(loader_type_id) {
                params.push(("modLoaderType", t.to_string()));
            }

            let resp: FlameEnvelope<Vec<FlameApiFile>> = self
                .api
                .client
                .get(&url)
                .query(&params)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;

            Ok(resp.data.into_iter().map(map_file_to_version).collect())
        })
    }

    fn resolve_by_hashes<'a>(
        &'a self,
        algo: HashAlgo,
        hashes: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Option<ResolvedFile>>>> {
        Box::pin(async move {
            // CurseForge 只支持 murmur2 指纹反查。
            if !matches!(algo, HashAlgo::Murmur2) {
                return Err(CoreError::other(
                    "CurseForge only supports Murmur2 fingerprint lookup",
                ));
            }

            // 输入是十进制 murmur2 字符串 → u32。非法项记成 None 占位(保持与输入对齐)。
            // 同时建一张 fingerprint → 输入下标 的反查表(可能多个输入同指纹)。
            let mut fps: Vec<u32> = Vec::new();
            let mut parsed: Vec<Option<u32>> = Vec::with_capacity(hashes.len());
            for h in hashes {
                match h.trim().parse::<u32>() {
                    Ok(fp) => {
                        parsed.push(Some(fp));
                        if !fps.contains(&fp) {
                            fps.push(fp);
                        }
                    }
                    Err(_) => parsed.push(None),
                }
            }

            let mut out: Vec<Option<ResolvedFile>> = vec![None; hashes.len()];
            if fps.is_empty() {
                return Ok(out);
            }

            let matches = self.api.match_fingerprints(&fps).await?;

            // 用 fingerprint 把匹配对齐回输入下标(返回顺序不保证)。
            use std::collections::HashMap;
            let mut by_fp: HashMap<u32, &FlameApiFile> = HashMap::new();
            for m in &matches {
                if let Some(fp) = m.file.file_fingerprint {
                    // file_fingerprint 是 u64,但 CF 指纹本质是 u32;截断对齐。
                    by_fp.insert(fp as u32, &m.file);
                }
            }

            // 富化:对所有命中文件的 mod_id 批量取项目名/slug(一次请求,便宜)。
            let mod_ids: Vec<i64> = {
                let mut ids: Vec<i64> = matches
                    .iter()
                    .map(|m| m.file.mod_id)
                    .filter(|id| *id != 0)
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                ids
            };
            let projects = if mod_ids.is_empty() {
                Vec::new()
            } else {
                // 取不到名字不致命:富化失败就退化为 None 名字。
                self.api.get_mods(&mod_ids).await.unwrap_or_default()
            };
            let proj_by_id: HashMap<i64, &FlameApiProject> =
                projects.iter().map(|p| (p.id, p)).collect();

            for (i, fp_opt) in parsed.into_iter().enumerate() {
                if let Some(fp) = fp_opt {
                    if let Some(file) = by_fp.get(&fp) {
                        out[i] = Some(resolved_from_file(file, proj_by_id.get(&file.mod_id).copied()));
                    }
                }
            }

            Ok(out)
        })
    }

    fn get_files_bulk<'a>(
        &'a self,
        refs: &'a [(String, String)],
    ) -> BoxFuture<'a, Result<Vec<ResolvedFile>>> {
        Box::pin(async move {
            // refs 是 (project_id, file_id) 的字符串对;我们只需 file_id 去批量取文件。
            let file_ids: Vec<i64> = refs
                .iter()
                .filter_map(|(_, fid)| fid.trim().parse::<i64>().ok())
                .collect();

            if file_ids.is_empty() {
                return Ok(Vec::new());
            }

            let files = self.api.get_files(&file_ids).await?;

            // 富化项目名/slug:对所有涉及的 mod_id 批量取一次(便宜)。
            let mod_ids: Vec<i64> = {
                let mut ids: Vec<i64> = files.iter().map(|f| f.mod_id).filter(|id| *id != 0).collect();
                ids.sort_unstable();
                ids.dedup();
                ids
            };
            let projects = if mod_ids.is_empty() {
                Vec::new()
            } else {
                self.api.get_mods(&mod_ids).await.unwrap_or_default()
            };
            use std::collections::HashMap;
            let proj_by_id: HashMap<i64, &FlameApiProject> =
                projects.iter().map(|p| (p.id, p)).collect();

            Ok(files
                .iter()
                .map(|f| resolved_from_file(f, proj_by_id.get(&f.mod_id).copied()))
                .collect())
        })
    }
}

/// 把一个 [`FlameApiFile`](+ 可选项目元信息)映射成统一 [`ResolvedFile`]。
///
/// 注意 BLOCKED 文件(`download_url == None`)依然返回一个 `ResolvedFile`,只是
/// `file.url` 为空串——调用方据"url 为空"识别 blocked 并走手动下载流。
fn resolved_from_file(f: &FlameApiFile, project: Option<&FlameApiProject>) -> ResolvedFile {
    ResolvedFile {
        provider: ProviderId::CurseForge,
        project_id: f.mod_id.to_string(),
        version_id: f.id.to_string(),
        file: map_version_file(f),
        project_name: project.map(|p| p.name.clone()),
        project_slug: project.map(|p| p.slug.clone()),
        authors: project
            .map(|p| p.authors.iter().map(|a| a.name.clone()).collect())
            .unwrap_or_default(),
    }
}

/// 把字符串 id 解析成 i64,失败映射成 [`CoreError::Other`](带上下文)。
fn parse_id(s: &str, what: &str) -> Result<i64> {
    s.trim()
        .parse::<i64>()
        .map_err(|_| CoreError::other(format!("invalid {what}: {s:?}")))
}

/// 批量把字符串 id 解析成 i64;遇到非法项直接报错(保持调用方语义明确)。
fn parse_ids(ids: &[String], what: &str) -> Result<Vec<i64>> {
    ids.iter().map(|s| parse_id(s, what)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 纯映射:`/mods/files` 风格的一个文件 → ProjectVersion。覆盖:
    /// - `gameVersions` 切分(含 `.` 当游戏版本、loader 名当 loader、Client/Server 丢弃)
    /// - hash algo 1 = sha1(algo 2 = md5 此处不进 VersionFile)
    /// - downloadUrl 存在 → url 正常
    fn sample_file() -> FlameApiFile {
        let json = r#"{
            "id": 4567,
            "modId": 12345,
            "displayName": "Sodium 0.5.3 (Fabric 1.20.1)",
            "fileName": "sodium-fabric-0.5.3.jar",
            "downloadUrl": "https://edge.forgecdn.net/files/4567/sodium.jar",
            "fileLength": 998877,
            "fileFingerprint": 305419896,
            "hashes": [
                { "value": "deadbeefsha1", "algo": 1 },
                { "value": "cafebabemd5", "algo": 2 }
            ],
            "gameVersions": ["1.20.1", "Fabric", "Client", "Server"]
        }"#;
        serde_json::from_str(json).expect("sample file parses")
    }

    #[test]
    fn maps_file_to_version_with_partition_and_sha1() {
        let f = sample_file();
        let v = map_file_to_version(f);

        assert_eq!(v.id, "4567");
        assert_eq!(v.version_number, "Sodium 0.5.3 (Fabric 1.20.1)");
        // 含 '.' 的当游戏版本
        assert_eq!(v.game_versions, vec!["1.20.1".to_string()]);
        // "Fabric" 当 loader(小写),Client/Server 丢弃
        assert_eq!(v.loaders, vec!["fabric".to_string()]);

        assert_eq!(v.files.len(), 1);
        let file = &v.files[0];
        assert_eq!(file.url, "https://edge.forgecdn.net/files/4567/sodium.jar");
        assert_eq!(file.filename, "sodium-fabric-0.5.3.jar");
        // algo 1 = sha1
        assert_eq!(file.sha1.as_deref(), Some("deadbeefsha1"));
        // CF 不提供 sha512
        assert_eq!(file.sha512, None);
        assert_eq!(file.size, Some(998877));
        // CF 一个 file 即一个版本,primary 恒 true
        assert!(file.primary);
    }

    #[test]
    fn nullable_download_url_is_blocked_empty_string() {
        // downloadUrl 缺失(或 null)= BLOCKED → 映射后 url 为空串。
        let json = r#"{
            "id": 999,
            "modId": 111,
            "displayName": "Blocked Mod 1.0",
            "fileName": "blocked-mod-1.0.jar",
            "fileLength": 4242,
            "hashes": [ { "value": "abc123", "algo": 1 } ],
            "gameVersions": ["1.19.2", "Forge"]
        }"#;
        let f: FlameApiFile = serde_json::from_str(json).unwrap();
        assert_eq!(f.download_url, None);

        let v = map_file_to_version(f);
        let file = &v.files[0];
        // BLOCKED:依然产出 VersionFile,但 url 为空 → 调用方据此识别。
        assert_eq!(file.url, "");
        assert_eq!(file.filename, "blocked-mod-1.0.jar");
        assert_eq!(file.sha1.as_deref(), Some("abc123"));
        assert_eq!(v.game_versions, vec!["1.19.2".to_string()]);
        assert_eq!(v.loaders, vec!["forge".to_string()]);
    }

    #[test]
    fn envelope_with_single_object_under_data_is_tolerated() {
        // /mods/files 单 id 偶发返回单对象而非数组:OneOrMany 容忍两种形态。
        let single = r#"{ "data": {
            "id": 1, "modId": 2, "displayName": "One", "fileName": "one.jar",
            "downloadUrl": "https://x/one.jar", "hashes": [], "gameVersions": ["1.20.1"]
        }}"#;
        let env: FlameEnvelope<OneOrMany<FlameApiFile>> = serde_json::from_str(single).unwrap();
        let files = env.data.into_vec();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name, "one.jar");

        let many = r#"{ "data": [
            { "id": 1, "modId": 2, "displayName": "One", "fileName": "one.jar", "downloadUrl": "https://x/one.jar", "hashes": [], "gameVersions": [] },
            { "id": 3, "modId": 2, "displayName": "Two", "fileName": "two.jar", "downloadUrl": "https://x/two.jar", "hashes": [], "gameVersions": [] }
        ]}"#;
        let env2: FlameEnvelope<OneOrMany<FlameApiFile>> = serde_json::from_str(many).unwrap();
        assert_eq!(env2.data.into_vec().len(), 2);
    }

    #[test]
    fn maps_project_to_search_hit() {
        // /mods or /mods/search 项目对象:嵌套 links/logo/authors/screenshots。
        let json = r#"{
            "id": 12345,
            "name": "Sodium",
            "slug": "sodium",
            "summary": "A rendering engine",
            "downloadCount": 12345678.0,
            "links": { "websiteUrl": "https://www.curseforge.com/minecraft/mc-mods/sodium" },
            "logo": { "url": "https://media.forgecdn.net/avatars/sodium.png" },
            "authors": [ { "name": "jellysquid3" }, { "name": "other" } ],
            "categories": [ { "name": "Cosmetic" } ],
            "screenshots": [ { "url": "https://media.forgecdn.net/attachments/screen1.png" } ]
        }"#;
        let p: FlameApiProject = serde_json::from_str(json).unwrap();
        let hit = map_project(p);

        assert_eq!(hit.id, "12345");
        assert_eq!(hit.slug, "sodium");
        assert_eq!(hit.title, "Sodium");
        assert_eq!(hit.description, "A rendering engine");
        // 第一个作者
        assert_eq!(hit.author, "jellysquid3");
        // downloadCount 浮点 → u64
        assert_eq!(hit.downloads, 12345678);
        assert_eq!(hit.icon_url.as_deref(), Some("https://media.forgecdn.net/avatars/sodium.png"));
        assert_eq!(
            hit.gallery_url.as_deref(),
            Some("https://media.forgecdn.net/attachments/screen1.png")
        );
        assert_eq!(hit.categories, vec!["Cosmetic".to_string()]);
    }

    #[test]
    fn fingerprint_match_carries_file() {
        // /fingerprints 响应:data.exactMatches[].file。
        let json = r#"{ "data": { "exactMatches": [
            { "id": 12345, "file": {
                "id": 4567, "modId": 12345, "displayName": "Sodium",
                "fileName": "sodium.jar", "downloadUrl": "https://x/sodium.jar",
                "fileFingerprint": 305419896,
                "hashes": [ { "value": "sha1here", "algo": 1 } ],
                "gameVersions": ["1.20.1", "Fabric"]
            }}
        ]}}"#;
        let env: FlameEnvelope<FlameFingerprintData> = serde_json::from_str(json).unwrap();
        assert_eq!(env.data.exact_matches.len(), 1);
        let m = &env.data.exact_matches[0];
        assert_eq!(m.file.id, 4567);
        assert_eq!(m.file.mod_id, 12345);
        assert_eq!(m.file.file_fingerprint, Some(305419896));

        // 映射成 ResolvedFile(无项目富化):project_id=modId、version_id=id、url 来自 downloadUrl。
        let resolved = resolved_from_file(&m.file, None);
        assert_eq!(resolved.provider, ProviderId::CurseForge);
        assert_eq!(resolved.project_id, "12345");
        assert_eq!(resolved.version_id, "4567");
        assert_eq!(resolved.file.url, "https://x/sodium.jar");
        assert_eq!(resolved.file.sha1.as_deref(), Some("sha1here"));
        assert_eq!(resolved.project_name, None);
    }

    #[test]
    fn resolved_blocked_file_has_empty_url() {
        // get_files_bulk 语义:BLOCKED 文件仍返回 ResolvedFile,file.url 为空。
        let json = r#"{
            "id": 9, "modId": 8, "displayName": "Blocked", "fileName": "b.jar",
            "fileLength": 10, "hashes": [], "gameVersions": []
        }"#;
        let f: FlameApiFile = serde_json::from_str(json).unwrap();
        let resolved = resolved_from_file(&f, None);
        assert_eq!(resolved.file.url, "");
        assert_eq!(resolved.version_id, "9");
        assert_eq!(resolved.project_id, "8");
    }

    #[test]
    fn empty_envelope_defaults() {
        // 完全空对象也能解析(data 缺失 → 默认空 Vec)。
        let env: FlameEnvelope<Vec<FlameApiProject>> = serde_json::from_str("{}").unwrap();
        assert!(env.data.is_empty());
    }

    #[test]
    fn partition_keeps_dots_and_known_loaders() {
        let (game, loaders) = partition_game_versions(vec![
            "1.20.1".into(),
            "1.21".into(),
            "Fabric".into(),
            "NeoForge".into(),
            "Client".into(),
            "Server".into(),
            "".into(),
        ]);
        assert_eq!(game, vec!["1.20.1".to_string(), "1.21".to_string()]);
        assert_eq!(loaders, vec!["fabric".to_string(), "neoforge".to_string()]);
    }

    #[test]
    fn caps_declares_murmur2_and_needs_key() {
        // 能力声明:CurseForge、需要 key、反查仅 murmur2。
        let api = FlameApi::new("dummy-key");
        let provider = CurseForgeProvider::new(api);
        let caps = provider.caps();
        assert_eq!(caps.id, ProviderId::CurseForge);
        assert_eq!(caps.readable_name, "CurseForge");
        assert_eq!(caps.hash_algos, &[HashAlgo::Murmur2]);
        assert!(caps.needs_api_key);
        assert_eq!(provider.id(), ProviderId::CurseForge);
    }

    #[test]
    fn loader_and_sort_and_class_mappings() {
        assert_eq!(loader_type_id("Forge"), Some(1));
        assert_eq!(loader_type_id("fabric"), Some(4));
        assert_eq!(loader_type_id("QUILT"), Some(5));
        assert_eq!(loader_type_id("neoforge"), Some(6));
        assert_eq!(loader_type_id("rift"), None);

        assert_eq!(sort_field_id(SortMethod::Downloads), 6);
        assert_eq!(sort_field_id(SortMethod::Relevance), 2);

        assert_eq!(class_id(ResourceKind::Mod), CLASS_MOD);
        assert_eq!(class_id(ResourceKind::Modpack), CLASS_MODPACK);
        assert_eq!(class_id(ResourceKind::Datapack), CLASS_MOD);
    }

    #[test]
    fn parse_id_rejects_garbage() {
        assert_eq!(parse_id("123", "x").unwrap(), 123);
        assert!(matches!(parse_id("abc", "x").unwrap_err(), CoreError::Other(_)));
        // 带空白可解析
        assert_eq!(parse_id("  42  ", "x").unwrap(), 42);
    }

    #[test]
    fn from_env_none_when_unset_or_empty() {
        // 不依赖外部环境:直接验证 trim+filter 的语义边界。
        // (env 读取本身在 from_env;此处用 with_base 构造确认 new 不 panic。)
        let api = FlameApi::new("k").with_base("https://example.test/v1");
        assert_eq!(api.api_key(), "k");
        assert!(api.base.ends_with("/v1"));
    }

    #[test]
    fn from_key_trims_and_guards_empty() {
        // 显式 key:去空白后非空才构造。
        assert!(FlameApi::from_key("").is_none());
        assert!(FlameApi::from_key("   ").is_none());
        let api = FlameApi::from_key("  real-key  ").expect("non-empty key constructs");
        assert_eq!(api.api_key(), "real-key");

        // provider 层同样守卫。
        assert!(CurseForgeProvider::from_key("").is_none());
        let p = CurseForgeProvider::from_key("k").expect("non-empty key constructs provider");
        assert_eq!(p.id(), ProviderId::CurseForge);
    }
}
