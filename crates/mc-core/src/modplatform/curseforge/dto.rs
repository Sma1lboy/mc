use super::*;

// ============================ 原始 json 承接类型 ============================

/// 通用响应信封:CurseForge 所有端点都把负载裹在 `{"data": …}` 里。
///
/// `data` 走 `#[serde(default)]` 容忍缺失;serde derive 不会自动给泛型加 `Default`
/// 约束,故用容器级 `bound` 显式声明 `T: Default + Deserialize`,使 `data` 缺失时回退
/// 到 `T::default()`(`Vec`/`OneOrMany`/`FlameFingerprintData` 都实现了 `Default`)。
#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: Default + serde::Deserialize<'de>"))]
pub(crate) struct FlameEnvelope<T> {
    #[serde(default)]
    pub(crate) data: T,
}

/// 容忍"单对象 or 数组"的反序列化包装。`/mods/files` 在 fileIds 只有一个时偶发返回
/// 单个对象而非数组;用 `#[serde(untagged)]` 同时接受两种形态。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum OneOrMany<T> {
    Many(Vec<T>),
    One(T),
}

impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}

impl<T> OneOrMany<T> {
    pub(crate) fn into_vec(self) -> Vec<T> {
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
    #[serde(default)]
    pub wiki_url: Option<String>,
    #[serde(default)]
    pub issues_url: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FlameApiLogo {
    #[serde(default)]
    pub url: Option<String>,
    /// 截图标题/描述(logo 上通常为空;详情页画廊用)。
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
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
pub(crate) fn partition_game_versions(raw: Vec<String>) -> (Vec<String>, Vec<String>) {
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
pub(crate) fn map_version_file(f: &FlameApiFile) -> VersionFile {
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
pub(crate) fn map_file_to_version(f: FlameApiFile) -> ProjectVersion {
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
pub(crate) fn map_project(p: FlameApiProject) -> SearchHit {
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

/// CF 项目 + description HTML → 统一的 [`ProjectDetail`] 渲染模型(与 Modrinth 同一份,
/// 前端不感知平台)。CF 没有 followers 概念 → 0;discord 链接 CF API 不提供 → None。
pub(crate) fn map_project_detail(
    p: FlameApiProject,
    body: String,
) -> crate::modplatform::modrinth::ProjectDetail {
    let downloads = if p.download_count.is_finite() && p.download_count > 0.0 {
        p.download_count as u64
    } else {
        0
    };
    crate::modplatform::modrinth::ProjectDetail {
        id: p.id.to_string(),
        slug: p.slug,
        title: p.name,
        description: p.summary,
        body,
        downloads,
        followers: 0,
        icon_url: p.logo.and_then(|l| l.url),
        categories: p.categories.into_iter().map(|c| c.name).collect(),
        gallery: p
            .screenshots
            .into_iter()
            .filter_map(|s| {
                s.url.map(|url| crate::modplatform::modrinth::GalleryImage {
                    url,
                    title: s.title,
                    description: s.description,
                    featured: false,
                })
            })
            .collect(),
        source_url: p.links.source_url.filter(|s| !s.is_empty()),
        issues_url: p.links.issues_url.filter(|s| !s.is_empty()),
        wiki_url: p.links.wiki_url.filter(|s| !s.is_empty()),
        discord_url: None,
    }
}
