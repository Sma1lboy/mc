use super::*;
use crate::modplatform::HashAlgo;

// ============================ 原始 json 承接类型 ============================

/// `/v2/search` 的顶层响应。
#[derive(Debug, Deserialize)]
pub(crate) struct RawSearchResponse {
    #[serde(default)]
    pub(crate) hits: Vec<RawSearchHit>,
}

/// 搜索结果中的一条 hit。字段名遵循 Modrinth `search` 端点(注意它和
/// `project` 端点字段不完全一样:这里 id 叫 `project_id`,作者叫 `author`)。
#[derive(Debug, Deserialize)]
pub(crate) struct RawSearchHit {
    #[serde(default)]
    pub(crate) project_id: String,
    #[serde(default)]
    pub(crate) slug: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) author: String,
    #[serde(default)]
    pub(crate) downloads: u64,
    #[serde(default)]
    pub(crate) icon_url: Option<String>,
    /// Modrinth search hits carry the featured gallery image and the full gallery
    /// URL list; either gives a high-res landscape cover.
    #[serde(default)]
    pub(crate) featured_gallery: Option<String>,
    #[serde(default)]
    pub(crate) gallery: Vec<String>,
    #[serde(default)]
    pub(crate) categories: Vec<String>,
    #[serde(default)]
    pub(crate) client_side: Option<String>,
    #[serde(default)]
    pub(crate) server_side: Option<String>,
}

/// `/v2/project/{id}` 的项目对象。这里 id 字段就叫 `id`,且**没有** `author`
/// 字段(作者要另算端点),所以作者留空。
#[derive(Debug, Deserialize)]
pub(crate) struct RawProject {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) slug: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) downloads: u64,
    #[serde(default)]
    pub(crate) icon_url: Option<String>,
    #[serde(default)]
    pub(crate) categories: Vec<String>,
    #[serde(default)]
    pub(crate) client_side: Option<String>,
    #[serde(default)]
    pub(crate) server_side: Option<String>,
    // —— 详情页额外字段(map_project 不消费,project_details 用)——
    /// 完整介绍正文(markdown 原文)。
    #[serde(default)]
    pub(crate) body: String,
    #[serde(default)]
    pub(crate) followers: u64,
    #[serde(default)]
    pub(crate) gallery: Vec<RawGalleryImage>,
    #[serde(default)]
    pub(crate) source_url: Option<String>,
    #[serde(default)]
    pub(crate) issues_url: Option<String>,
    #[serde(default)]
    pub(crate) wiki_url: Option<String>,
    #[serde(default)]
    pub(crate) discord_url: Option<String>,
}

/// `/v2/project/{id}` 的 `gallery` 数组里的一张图。
#[derive(Debug, Deserialize)]
pub(crate) struct RawGalleryImage {
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) featured: bool,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
    /// 展示排序(升序)。
    #[serde(default)]
    pub(crate) ordering: i64,
}

/// `/v2/project/{id}/version` 数组里的一个版本。
#[derive(Debug, Deserialize)]
pub(crate) struct RawVersion {
    #[serde(default)]
    pub(crate) id: String,
    /// 该版本所属项目 id。`/version/{id}` 与 `/version_files` 的版本对象都带它,
    /// 用于哈希反查时构造 [`ResolvedFile::project_id`]。
    #[serde(default)]
    pub(crate) project_id: String,
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) version_number: String,
    #[serde(default)]
    pub(crate) game_versions: Vec<String>,
    #[serde(default)]
    pub(crate) loaders: Vec<String>,
    #[serde(default)]
    pub(crate) files: Vec<RawFile>,
    #[serde(default)]
    pub(crate) dependencies: Vec<RawDependency>,
    // —— 详情页额外字段(map_version 不消费,version_details 用)——
    #[serde(default)]
    pub(crate) version_type: String,
    #[serde(default)]
    pub(crate) date_published: String,
    #[serde(default)]
    pub(crate) downloads: u64,
    #[serde(default)]
    pub(crate) changelog: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawFile {
    #[serde(default)]
    pub(crate) url: String,
    #[serde(default)]
    pub(crate) filename: String,
    #[serde(default)]
    pub(crate) hashes: RawHashes,
    #[serde(default)]
    pub(crate) size: Option<u64>,
    #[serde(default)]
    pub(crate) primary: bool,
}

/// Modrinth 把校验和放在 `hashes` 对象里(`sha1` / `sha512`)。
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RawHashes {
    #[serde(default)]
    sha1: Option<String>,
    #[serde(default)]
    sha512: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RawDependency {
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) version_id: Option<String>,
    #[serde(default)]
    pub(crate) dependency_type: Option<String>,
}

// ============================ 纯映射函数(可单测) ============================

pub(crate) fn map_search_hit(r: RawSearchHit) -> SearchHit {
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
        client_side: ProjectSideSupport::from_modrinth(r.client_side.as_deref()),
        server_side: ProjectSideSupport::from_modrinth(r.server_side.as_deref()),
    }
}

pub(crate) fn map_project(r: RawProject) -> SearchHit {
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
        client_side: ProjectSideSupport::from_modrinth(r.client_side.as_deref()),
        server_side: ProjectSideSupport::from_modrinth(r.server_side.as_deref()),
    }
}

pub(crate) fn map_version(r: RawVersion) -> ProjectVersion {
    ProjectVersion {
        id: r.id,
        name: r.name,
        version_number: r.version_number,
        game_versions: r.game_versions,
        loaders: r.loaders,
        files: r.files.into_iter().map(map_file).collect(),
        dependencies: r.dependencies.into_iter().map(map_dependency).collect(),
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    }
}

/// 一个版本的展示用详情(整合包详情页:含 changelog / 发布时间 / 类型 / 下载数,
/// 以及该版本的 `.mrpack` 文件地址)。比统一的 [`ProjectVersion`] 多带 UI 信息。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
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

pub(crate) fn map_version_detail(r: RawVersion) -> VersionDetail {
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

/// 一个项目的展示详情(整合包详情页「简介」用):长描述正文(markdown)、
/// 画廊、外部链接等。比 [`SearchHit`] 多带 `body` / `gallery` / 链接,供详情页
/// 渲染一个像样的「简介」标签页(而不只是一句话 description)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ProjectDetail {
    pub id: String,
    pub slug: String,
    pub title: String,
    /// 一句话简介。
    pub description: String,
    /// 完整介绍正文(markdown 原文,前端渲染它)。
    pub body: String,
    pub downloads: u64,
    pub followers: u64,
    pub icon_url: Option<String>,
    pub categories: Vec<String>,
    /// 画廊图片(已按 `ordering` 升序;`featured` 为推荐封面)。
    pub gallery: Vec<GalleryImage>,
    pub source_url: Option<String>,
    pub issues_url: Option<String>,
    pub wiki_url: Option<String>,
    pub discord_url: Option<String>,
}

/// 项目画廊里的一张图。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct GalleryImage {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub featured: bool,
}

pub(crate) fn map_project_detail(r: RawProject) -> ProjectDetail {
    let mut imgs = r.gallery;
    imgs.sort_by_key(|g| g.ordering);
    let gallery = imgs
        .into_iter()
        .filter(|g| !g.url.is_empty())
        .map(|g| GalleryImage {
            url: g.url,
            title: g.title,
            description: g.description,
            featured: g.featured,
        })
        .collect();
    ProjectDetail {
        id: r.id,
        slug: r.slug,
        title: r.title,
        description: r.description,
        body: r.body,
        downloads: r.downloads,
        followers: r.followers,
        icon_url: r.icon_url,
        categories: r.categories,
        gallery,
        source_url: r.source_url,
        issues_url: r.issues_url,
        wiki_url: r.wiki_url,
        discord_url: r.discord_url,
    }
}

pub(crate) fn map_file(r: RawFile) -> VersionFile {
    VersionFile {
        url: r.url,
        filename: r.filename,
        sha1: r.hashes.sha1,
        sha512: r.hashes.sha512,
        size: r.size,
        primary: r.primary,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    }
}

pub(crate) fn map_dependency(r: RawDependency) -> Dependency {
    Dependency {
        project_id: r.project_id,
        version_id: r.version_id,
        // 缺省给 "required",这是 Modrinth 最常见且语义最保守的取值。
        dependency_type: r.dependency_type.unwrap_or_else(|| "required".to_string()),
    }
}


/// 在一个版本的文件里找出哈希(sha1/sha512)等于 `wanted` 的那一个。
/// 一个版本可能有多文件(主 jar、sources 等),反查命中的是某一个具体文件,
/// 必须按算法逐个比对哈希,而不能假定是主文件。
pub(crate) fn find_file_by_hash(version: &RawVersion, algo: HashAlgo, wanted: &str) -> Option<VersionFile> {
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
pub(crate) fn map_file_ref(r: &RawFile) -> VersionFile {
    VersionFile {
        url: r.url.clone(),
        filename: r.filename.clone(),
        sha1: r.hashes.sha1.clone(),
        sha512: r.hashes.sha512.clone(),
        size: r.size,
        primary: r.primary,
        client_side: ProjectSideSupport::Unknown,
        server_side: ProjectSideSupport::Unknown,
    }
}
