use super::*;

// ============================ facets / query 构造 ============================

/// 把统一 [`SortMethod`] 映射到 Modrinth `/search` 的 `index` 参数。
/// Modrinth 支持:`relevance` / `downloads` / `follows` / `newest` / `updated`。
/// `Newest` → `newest`(按创建时间),`Updated` → `updated`(按最近更新),默认 `relevance`。
pub(crate) fn modrinth_index(sort: SortMethod) -> &'static str {
    match sort {
        SortMethod::Relevance => "relevance",
        SortMethod::Downloads => "downloads",
        SortMethod::Newest => "newest",
        SortMethod::Updated => "updated",
    }
}

/// 构造 [`build_facets`] 的输入:把单值兼容字段与多选 facet 字段都收拢到一处,
/// 单值入口([`Self::single`])与完整查询入口([`Self::from_query`])复用同一构造逻辑。
#[derive(Debug, Default)]
pub(crate) struct FacetSelection {
    pub(crate) kind: Option<ResourceKind>,
    /// 游戏版本并集(单值 `game_version` + 多选 `game_versions`,去重)。
    pub(crate) game_versions: Vec<String>,
    /// loader 并集(单值 `loader` + 多选 `loaders`,各经 `accepted_loaders` 展开后去重)。
    pub(crate) loaders: Vec<String>,
    /// 内容分类(各自成一个 AND 组)。
    pub(crate) categories: Vec<String>,
    /// 运行环境:`"client"` / `"server"`(其余忽略)。
    pub(crate) environment: Option<String>,
    /// 仅开源项目。
    pub(crate) open_source: bool,
}

impl FacetSelection {
    /// 单值兼容入口(旧 `search` / `search_sorted` 用):只有 kind + 单个游戏版本 + 单个 loader。
    pub(crate) fn single(kind: ResourceKind, game_version: Option<&str>, loader: Option<&str>) -> Self {
        let mut sel = Self { kind: Some(kind), ..Self::default() };
        if let Some(v) = game_version.filter(|s| !s.is_empty()) {
            sel.game_versions.push(v.to_string());
        }
        sel.add_loader(loader);
        sel
    }

    /// 完整查询入口:合并单值兼容字段与多选 facet 字段(并集 + 去重)。
    pub(crate) fn from_query(q: &SearchQuery) -> Self {
        let mut sel = Self { kind: Some(q.kind), ..Self::default() };

        // 游戏版本:单值 + 多选,去重保序。
        for v in q.game_version.iter().chain(q.game_versions.iter()) {
            push_unique(&mut sel.game_versions, v);
        }
        // loader:单值 + 多选,各经 accepted_loaders 展开(Quilt→quilt+fabric)后去重保序。
        sel.add_loader(q.loader.as_deref());
        for l in &q.loaders {
            sel.add_loader(Some(l.as_str()));
        }
        // 分类:去重保序。
        for c in &q.categories {
            push_unique(&mut sel.categories, c);
        }
        sel.environment = q.environment.as_deref().filter(|s| !s.is_empty()).map(str::to_string);
        sel.open_source = q.open_source.unwrap_or(false);
        sel
    }

    /// 把一个 loader 经 [`crate::modplatform::accepted_loaders`] 展开后并入 `self.loaders`(去重保序)。
    fn add_loader(&mut self, loader: Option<&str>) {
        if let Some(l) = loader.filter(|s| !s.is_empty()) {
            for accepted in crate::modplatform::accepted_loaders(l) {
                push_unique(&mut self.loaders, &accepted);
            }
        }
    }
}

/// 追加到 `vec`,跳过空串与已存在项(保序去重)。
fn push_unique(vec: &mut Vec<String>, item: &str) {
    if !item.is_empty() && !vec.iter().any(|x| x == item) {
        vec.push(item.to_string());
    }
}

/// 构造 Modrinth `facets` 参数(一个 json 字符串)。
///
/// facets 是 "AND of OR" 的二维数组,形如
/// `[["project_type:mod"],["categories:base"],["categories:fabric","categories:forge"],["versions:1.20.1","versions:1.21"]]`:
/// 外层各组之间是 AND,内层各项之间是 OR。映射规则:
/// - `project_type:<kind>` —— 始终一个组。
/// - 每个**内容分类**各成一个 AND 组(`["categories:<name>"]`),多选即 AND(都得带)。
/// - 所有 **loader** 合成一个 OR 组(`["categories:fabric","categories:forge",…]`)。
/// - 所有**游戏版本**合成一个 OR 组(`["versions:1.20.1","versions:1.21",…]`)。
/// - **环境**:`client` → `["client_side:optional","client_side:required"]`;
///   `server` → `["server_side:optional","server_side:required"]`。
///
/// 数据包(`ResourceKind::Datapack`)在 Modrinth 是 `mod` 项目 + `datapack` category,
/// 故额外追加 `["categories:datapack"]`。
pub(crate) fn build_facets(sel: &FacetSelection) -> String {
    let mut groups: Vec<String> = Vec::new();

    if let Some(kind) = sel.kind {
        groups.push(facet_group(&[&format!("project_type:{}", kind.as_modrinth_project_type())]));
        if matches!(kind, ResourceKind::Datapack) {
            groups.push(facet_group(&["categories:datapack"]));
        }
    }

    // 每个内容分类各成一个 AND 组。
    for c in &sel.categories {
        groups.push(facet_group(&[&format!("categories:{c}")]));
    }

    // 所有 loader 合成一个 OR 组。
    if !sel.loaders.is_empty() {
        let items: Vec<String> = sel.loaders.iter().map(|x| format!("categories:{x}")).collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        groups.push(facet_group(&refs));
    }

    // 所有游戏版本合成一个 OR 组。
    if !sel.game_versions.is_empty() {
        let items: Vec<String> = sel.game_versions.iter().map(|x| format!("versions:{x}")).collect();
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        groups.push(facet_group(&refs));
    }

    // 环境:client/server 各自是一个 OR 组(optional 或 required 都算)。
    if let Some(env) = sel.environment.as_deref() {
        match env {
            "client" => groups.push(facet_group(&["client_side:optional", "client_side:required"])),
            "server" => groups.push(facet_group(&["server_side:optional", "server_side:required"])),
            _ => {}
        }
    }

    // 仅开源:open_source:true 单独一个组。
    if sel.open_source {
        groups.push(facet_group(&["open_source:true"]));
    }

    format!("[{}]", groups.join(","))
}

/// 把一组 facet 字符串拼成内层 OR 组,如 `["a:b","c:d"]`,每项做 json 字符串转义。
fn facet_group(items: &[&str]) -> String {
    let inner: Vec<String> = items.iter().map(|s| json_quote(s)).collect();
    format!("[{}]", inner.join(","))
}

/// 把一组字符串编码成 json 数组字符串,如 `["fabric"]`,用于 loaders/game_versions 参数。
pub(crate) fn json_string_array(items: &[&str]) -> String {
    let inner: Vec<String> = items.iter().map(|s| json_quote(s)).collect();
    format!("[{}]", inner.join(","))
}

/// 用 serde_json 给单个字符串做带引号的 json 转义(保证特殊字符安全)。
fn json_quote(s: &str) -> String {
    // serde_json::to_string 对 &str 永不失败(字符串总能序列化),unwrap 安全。
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

// ============================ facet 分类法(tags) ============================

/// 进程内缓存:Modrinth 的 tag 端点(分类 / loader / 游戏版本)极少变动,首次拉取后
/// 缓存到进程结束,避免每次打开浏览页都打三次网络。仅用于**默认 base** 的客户端;
/// 自定义 base(测试 / 镜像)绕过缓存,见 [`ModrinthApi::content_facets`]。
pub(crate) static FACET_TAGS_CACHE: tokio::sync::OnceCell<FacetTagsDto> = tokio::sync::OnceCell::const_new();

/// Modrinth 的 facet 分类法:内容分类 / loader / 游戏版本。前端据此渲染过滤面板。
/// 注意:这些是平台动态数据(分类名直接来自 Modrinth),**不**走 i18n,原样展示。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct FacetTagsDto {
    pub categories: Vec<CategoryTag>,
    pub loaders: Vec<LoaderTag>,
    pub game_versions: Vec<GameVersionTag>,
}

/// 一个内容分类(`GET /tag/category` 的一项)。`header` 把分类分组
/// (`categories` / `features` / `resolutions` / `performance impact`);
/// `project_type` 指出该分类适用于哪个资源类型(`mod` / `modpack` / `shader` / …)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct CategoryTag {
    pub name: String,
    pub header: String,
    pub project_type: String,
}

/// 一个 loader(`GET /tag/loader` 的一项)。`supported_project_types` 指出该 loader
/// 适用于哪些资源类型(过滤面板据此只在相关 kind 下显示对应 loader)。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct LoaderTag {
    pub name: String,
    pub supported_project_types: Vec<String>,
}

/// 一个游戏版本(`GET /tag/game_version` 的一项)。`version_type` 区分
/// `release` / `snapshot` / `alpha` / `beta`,前端默认可只展示 release。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct GameVersionTag {
    pub version: String,
    pub version_type: String,
}

/// `GET /tag/category` 的一项原始 json。
#[derive(Debug, Deserialize)]
pub(crate) struct RawCategoryTag {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) header: String,
    #[serde(default)]
    pub(crate) project_type: String,
}

/// `GET /tag/loader` 的一项原始 json。
#[derive(Debug, Deserialize)]
pub(crate) struct RawLoaderTag {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) supported_project_types: Vec<String>,
}

/// `GET /tag/game_version` 的一项原始 json。
#[derive(Debug, Deserialize)]
pub(crate) struct RawGameVersionTag {
    #[serde(default)]
    pub(crate) version: String,
    #[serde(default)]
    pub(crate) version_type: String,
}

pub(crate) fn map_category_tag(r: RawCategoryTag) -> CategoryTag {
    CategoryTag { name: r.name, header: r.header, project_type: r.project_type }
}

pub(crate) fn map_loader_tag(r: RawLoaderTag) -> LoaderTag {
    LoaderTag { name: r.name, supported_project_types: r.supported_project_types }
}

pub(crate) fn map_game_version_tag(r: RawGameVersionTag) -> GameVersionTag {
    GameVersionTag { version: r.version, version_type: r.version_type }
}
