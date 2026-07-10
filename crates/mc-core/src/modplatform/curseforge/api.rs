use super::*;

impl FlameApi {
    /// 用给定 API key 构造一个新客户端。
    ///
    /// `x-api-key` 与 UA 固化进默认头。复用按 key 缓存的进程级 [`shared_client`],同一 key
    /// 的多个 `FlameApi` 共享同一 TLS/连接池。构造失败(几乎不会:仅 TLS 后端初始化失败或
    /// header 含非法字节)走 `expect`——属于环境级灾难,失败即代表无法发任何请求。
    pub fn new(api_key: impl Into<String>) -> Self {
        let api_key = api_key.into();
        let client = shared_client(&api_key);
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

    /// 取一个项目的完整详情(长描述 + 画廊 + 外部链接),映射成与 Modrinth 同一份
    /// [`ProjectDetail`] 渲染模型(详情页「简介」标签用)。两次请求:`POST /mods`(元信息)
    /// 加 `GET /mods/{id}/description`(HTML 正文;CF 的 body 是 HTML 而非 markdown,
    /// 前端渲染器转义 + 白名单重建,两种输入都安全)。
    pub async fn project_details(&self, id: i64) -> Result<crate::modplatform::modrinth::ProjectDetail> {
        let mods = self.get_mods(&[id]).await?;
        let m = mods
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::other(format!("CurseForge project {id} not found")))?;
        let body = self.get_description(id).await.unwrap_or_default();
        Ok(map_project_detail(m, body))
    }

    /// `GET /mods/{id}/description` → HTML 字符串(`{"data":"<p>…</p>"}`)。
    async fn get_description(&self, id: i64) -> Result<String> {
        let url = format!("{}/mods/{}/description", self.base, id);
        let resp: FlameEnvelope<String> =
            self.client.get(&url).send().await?.error_for_status()?.json().await?;
        Ok(resp.data)
    }

    /// [`project_details`] 的本地持久缓存版,与 Modrinth 共享同一套「新鲜命中 → 抓取回写 →
    /// stale 回退」逻辑;缓存落在 `<cache_dir>/curseforge/project/<id>.json`。
    pub async fn project_details_cached(
        &self,
        id: i64,
        cache_dir: &std::path::Path,
        ttl: std::time::Duration,
    ) -> Result<crate::modplatform::modrinth::ProjectDetail> {
        crate::modplatform::modrinth::project_details_via_cache(
            cache_dir,
            "curseforge",
            &id.to_string(),
            ttl,
            || self.project_details(id),
        )
        .await
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
