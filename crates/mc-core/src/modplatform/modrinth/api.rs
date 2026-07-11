use super::*;

impl ModrinthApi {
    /// 构造一个新客户端。复用进程级共享的 [`shared_client`](内含固化 UA 的连接池),
    /// 多个 `ModrinthApi` 因此共享同一 TLS/连接池,免去每次重建带来的冷连接延迟。
    pub fn new() -> Self {
        Self { client: shared_client(), base: API_BASE.to_string() }
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
    ///
    /// 排序固定为相关度;需要其它排序走 [`Self::search_sorted`]。
    pub async fn search(
        &self,
        query: &str,
        kind: ResourceKind,
        game_version: Option<&str>,
        loader: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<SearchHit>> {
        self.search_sorted(query, kind, game_version, loader, limit, offset, SortMethod::Relevance)
            .await
    }

    /// 同 [`Self::search`],但显式指定排序方式。`sort` 映射到 Modrinth `index`
    /// (见 [`modrinth_index`])。
    #[allow(clippy::too_many_arguments)]
    pub async fn search_sorted(
        &self,
        query: &str,
        kind: ResourceKind,
        game_version: Option<&str>,
        loader: Option<&str>,
        limit: u32,
        offset: u32,
        sort: SortMethod,
    ) -> Result<Vec<SearchHit>> {
        let facets = build_facets(&FacetSelection::single(kind, game_version, loader));
        self.run_search(query, &facets, limit, offset, sort).await
    }

    /// 用完整 [`SearchQuery`] 搜索:把单值兼容字段(`game_version` / `loader`)与多选 facet
    /// 字段(`game_versions` / `loaders` / `categories` / `environment`)合并成正确的 Modrinth
    /// facets(见 [`build_facets`])。Discover 多选过滤经此路由。
    pub async fn search_query(&self, q: &SearchQuery) -> Result<Vec<SearchHit>> {
        let facets = build_facets(&FacetSelection::from_query(q));
        self.run_search(&q.text, &facets, q.limit, q.offset, q.sort).await
    }

    /// 共享的 `/search` 请求逻辑:已构造好的 `facets` 串 + 文本 + 分页 + 排序。
    async fn run_search(
        &self,
        query: &str,
        facets: &str,
        limit: u32,
        offset: u32,
        sort: SortMethod,
    ) -> Result<Vec<SearchHit>> {
        let limit = limit.clamp(1, 100);
        let url = format!("{}/search", self.base);
        let resp: RawSearchResponse = self
            .client
            .get(&url)
            .query(&[
                ("query", query),
                ("facets", facets),
                ("limit", &limit.to_string()),
                ("offset", &offset.to_string()),
                ("index", modrinth_index(sort)),
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
        // Quilt 实例同时接受 fabric 版本;其余 loader 返回单元素,查询与之前完全一致。
        let loaders_vec = loader.map(crate::modplatform::accepted_loaders).filter(|v| !v.is_empty());
        let loaders_param = loaders_vec.as_ref().map(|v| {
            let refs: Vec<&str> = v.iter().map(String::as_str).collect();
            json_string_array(&refs)
        });
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

    /// 便捷方法:解析 `/project/{id}` 的完整详情(含 body / gallery / 链接)。
    /// 与 [`Self::parse_project`] 同源字节,但保留详情页需要的全部字段。
    pub fn parse_project_detail(bytes: &[u8]) -> Result<ProjectDetail> {
        let raw: RawProject = serde_json::from_slice(bytes)
            .map_err(|e| CoreError::Parse { what: "modrinth project detail".into(), source: e })?;
        Ok(map_project_detail(raw))
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

    /// 按文件哈希批量查询"在给定 loader / 游戏版本下的最新版本"(`POST /v2/version_files/update`)。
    ///
    /// 这是更新检查的核心:对已装 mod 的每个文件 sha1,直接拿回 Modrinth 认为的最新
    /// 兼容版本(同 `/version/{id}` 形状)。响应同样是 *键为请求哈希* 的对象,未命中的
    /// 哈希缺席。请求体追加 `loaders` / `game_versions` 过滤,确保返回的"最新"确实兼容
    /// 当前实例;为空时不过滤(交给调用方约束)。
    pub async fn latest_versions_from_hashes(
        &self,
        hashes: &[String],
        algorithm: &str,
        loaders: &[String],
        game_versions: &[String],
    ) -> Result<std::collections::HashMap<String, ProjectVersion>> {
        let url = format!("{}/version_files/update", self.base);
        let body = serde_json::json!({
            "hashes": hashes,
            "algorithm": algorithm,
            "loaders": loaders,
            "game_versions": game_versions,
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
        let raw = Self::parse_raw_versions_from_hashes(&bytes)?;
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
    pub(crate) fn parse_raw_versions_from_hashes(
        bytes: &[u8],
    ) -> Result<std::collections::HashMap<String, RawVersion>> {
        serde_json::from_slice(bytes).map_err(|e| CoreError::Parse {
            what: "modrinth version_files response".into(),
            source: e,
        })
    }

    /// 同 [`Self::versions_from_hashes`],但返回保留 `project_id` 的原始版本对象。
    /// 哈希反查内部用——公开方法返回的统一 [`ProjectVersion`] 不带 project_id。
    pub(crate) async fn raw_versions_from_hashes(
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


impl ModrinthApi {
    /// 列出某项目所有版本的展示详情(含 changelog / 类型 / 发布时间 + `.mrpack` 地址)。
    /// 整合包详情页用。
    pub async fn version_details(&self, project_id: &str) -> Result<Vec<VersionDetail>> {
        let url = format!("{}/project/{}/version", self.base, project_id);
        let raws: Vec<RawVersion> =
            self.client.get(&url).send().await?.error_for_status()?.json().await?;
        Ok(raws.into_iter().map(map_version_detail).collect())
    }

    /// 取某项目的完整详情(长描述正文 + 画廊 + 外部链接)。详情页「简介」用。
    pub async fn project_details(&self, id: &str) -> Result<ProjectDetail> {
        let url = format!("{}/project/{}", self.base, id);
        let bytes =
            self.client.get(&url).send().await?.error_for_status()?.bytes().await?;
        Self::parse_project_detail(&bytes)
    }

    /// [`project_details`] 的本地持久缓存版:实例详情头部 + 「概览」标签每次打开都要这份数据,
    /// 不该每次都打 Modrinth。命中新鲜缓存(`< ttl`)直接返回;过期或无缓存则抓取并回写;
    /// **抓取失败时回退到旧缓存**(stale-while-error,离线也能显示上次的 logo/简介)。
    /// 缓存落在 `<cache_dir>/modrinth/project/<id>.json`,按 `id` 索引。
    pub async fn project_details_cached(
        &self,
        id: &str,
        cache_dir: &std::path::Path,
        ttl: std::time::Duration,
    ) -> Result<ProjectDetail> {
        project_details_via_cache(cache_dir, "modrinth", id, ttl, || self.project_details(id)).await
    }
}

/// 「新鲜缓存 → 抓取回写 → stale 回退」的共享取舍逻辑,Modrinth 与 CurseForge 的项目详情
/// 缓存都走这里(`<cache_dir>/<provider>/project/<id>.json`)。
pub(crate) async fn project_details_via_cache<F, Fut>(
    cache_dir: &std::path::Path,
    provider: &str,
    id: &str,
    ttl: std::time::Duration,
    fetch: F,
) -> Result<ProjectDetail>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<ProjectDetail>>,
{
    let path = project_cache_path(cache_dir, provider, id);
    if let Some(hit) = read_project_cache(&path, Some(ttl)) {
        return Ok(hit);
    }
    match fetch().await {
        Ok(fresh) => {
            write_project_cache(&path, &fresh);
            Ok(fresh)
        }
        // 网络/解析失败:有旧缓存就用旧的(忽略 ttl),否则把错误抛出去。
        Err(e) => read_project_cache(&path, None).ok_or(e),
    }
}

impl ModrinthApi {
    /// 取 Modrinth 的 facet 分类法(`/tag/category` + `/tag/loader` + `/tag/game_version`)。
    ///
    /// 默认 base 的客户端用进程内 [`FACET_TAGS_CACHE`] 缓存——这些 tag 极少变动,重复调用
    /// 不再打网络。自定义 base(测试 / 镜像)绕过缓存,直接拉取。
    pub async fn content_facets(&self) -> Result<FacetTagsDto> {
        if self.base == API_BASE {
            FACET_TAGS_CACHE.get_or_try_init(|| self.fetch_facets()).await.cloned()
        } else {
            self.fetch_facets().await
        }
    }

    /// 三个 tag 端点并发拉取并解析(无缓存)。失败映射成 [`CoreError::Network`] / [`CoreError::Parse`]。
    async fn fetch_facets(&self) -> Result<FacetTagsDto> {
        let cat_url = format!("{}/tag/category", self.base);
        let loader_url = format!("{}/tag/loader", self.base);
        let gv_url = format!("{}/tag/game_version", self.base);

        let (cat_bytes, loader_bytes, gv_bytes) = futures::try_join!(
            async { self.client.get(&cat_url).send().await?.error_for_status()?.bytes().await },
            async { self.client.get(&loader_url).send().await?.error_for_status()?.bytes().await },
            async { self.client.get(&gv_url).send().await?.error_for_status()?.bytes().await },
        )?;

        Self::parse_facets(&cat_bytes, &loader_bytes, &gv_bytes)
    }

    /// 纯解析:把三个 tag 端点的字节映射成 [`FacetTagsDto`](可单测)。
    pub fn parse_facets(
        categories: &[u8],
        loaders: &[u8],
        game_versions: &[u8],
    ) -> Result<FacetTagsDto> {
        let raw_cats: Vec<RawCategoryTag> = serde_json::from_slice(categories)
            .map_err(|e| CoreError::Parse { what: "modrinth tag/category".into(), source: e })?;
        let raw_loaders: Vec<RawLoaderTag> = serde_json::from_slice(loaders)
            .map_err(|e| CoreError::Parse { what: "modrinth tag/loader".into(), source: e })?;
        let raw_gvs: Vec<RawGameVersionTag> = serde_json::from_slice(game_versions)
            .map_err(|e| CoreError::Parse { what: "modrinth tag/game_version".into(), source: e })?;
        Ok(FacetTagsDto {
            categories: raw_cats.into_iter().map(map_category_tag).collect(),
            loaders: raw_loaders.into_iter().map(map_loader_tag).collect(),
            game_versions: raw_gvs.into_iter().map(map_game_version_tag).collect(),
        })
    }
}
