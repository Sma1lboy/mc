use super::*;

impl Downloader {
    /// 创建下载器。`concurrency` 为同时在飞的最大请求数。
    ///
    /// client 启用 gzip 透明解压、连接池、统一 user-agent。构造失败(极少见,
    /// 通常是 TLS 后端初始化问题)会映射为 [`CoreError::Network`]。
    pub fn new(concurrency: usize) -> Result<Self> {
        // concurrency 至少为 1,避免 Semaphore(0) 永久阻塞。
        let concurrency = concurrency.max(1);
        let client = reqwest::Client::builder()
            .user_agent("mc-launcher/0.1")
            .gzip(true)
            // 连接池保活:同一 host 的后续请求复用已建立的 TLS 连接。
            .pool_max_idle_per_host(concurrency)
            .pool_idle_timeout(Duration::from_secs(90))
            // 整体超时不设(大 jar 慢网可能很久),但连接阶段设上限以快速失败。
            .connect_timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            sem: Arc::new(Semaphore::new(concurrency)),
            concurrency,
            mirror: MirrorResolver::none(),
            cf_api_key: None,
        })
    }

    /// 构造时设定的并发度(测试用:验证它独立于在飞许可数)。
    #[cfg(test)]
    pub(crate) fn configured_concurrency(&self) -> usize {
        self.concurrency
    }

    /// 设置镜像改写器(链式)。默认无镜像(直连官方)。
    pub fn with_mirror(mut self, mirror: MirrorResolver) -> Self {
        self.mirror = mirror;
        self
    }

    /// 设置 CurseForge API key(链式)。去空白后非空才生效;仅在下载 CurseForge 自家
    /// host 时随 `x-api-key` 头发送(见 [`is_curseforge_host`])。`None`/空 = 不带 key。
    pub fn with_cf_api_key(mut self, key: Option<String>) -> Self {
        self.cf_api_key = key.and_then(|k| {
            let t = k.trim();
            if t.is_empty() { None } else { Some(t.to_string()) }
        });
        self
    }

    /// 给一个 GET 请求按 host 决定是否附加 CurseForge `x-api-key` 头。命中 CurseForge
    /// host 且持有 key 才加;其它 host 一律原样返回(绝不把 key 泄露给非 CF host)。
    pub(crate) fn apply_cf_auth(&self, req: reqwest::RequestBuilder, url: &str) -> reqwest::RequestBuilder {
        match self.cf_api_key.as_deref() {
            Some(key) if url_is_curseforge(url) => req.header("x-api-key", key),
            _ => req,
        }
    }

    /// 暴露底层 client,供其它模块(如 auth)复用同一连接池。
    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// 下载单个文件:幂等跳过 -> 候选源失败转移(每源带重试)-> 强校验 -> 原子落盘。
    ///
    /// 候选源 = 主源 + 各镜像,经镜像改写展开去重(见 [`Self::candidate_urls`])。任一候选
    /// 成功且校验通过即返回;某候选 404 / 校验不符 / 重试耗尽则换下一候选(坏镜像最常见,
    /// 故校验不符也换源而非直接失败)。全部候选用尽才返回最后一个错误。
    pub async fn download_one(&self, item: &DownloadItem) -> Result<()> {
        let check = item.checksum();

        // 1) 已存在且校验通过则跳过,使重试整批时不重复下载已完成项。
        if let Some(c) = &check {
            if c.verify(&item.path) {
                return Ok(());
            }
        }

        // 2) 确保父目录存在。
        if let Some(parent) = item.path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| CoreError::io(parent.to_path_buf(), e))?;
        }

        // 唯一临时名:两个实例并发把同一个库写进共享 store 时,各自拿到不同的 .part,
        // 不会互相截断字节、也不会在校验失败 remove_file 时删掉对方写到一半的文件。
        let part_path = crate::fs::unique_temp_sibling(&item.path, "part");
        let candidates = self.candidate_urls(item);
        let mut last_err: Option<CoreError> = None;

        for url in &candidates {
            // 每个候选源内部带指数退避重试;404 / 非可重试错误立刻换源。
            for attempt in 0..MAX_ATTEMPTS {
                match self.fetch_to_part(url, &part_path).await {
                    Ok(()) => {
                        // 强校验(如有)。不匹配 -> 删 .part,记错并换下一候选。
                        if let Some(c) = &check {
                            if !c.verify(&part_path) {
                                let _ = fs::remove_file(&part_path).await;
                                last_err = Some(CoreError::Checksum {
                                    path: item.path.clone(),
                                    expected: c.expected().to_string(),
                                    actual: "(mismatch)".into(),
                                });
                                tracing::debug!(url = %url, "checksum mismatch, trying next source");
                                break;
                            }
                        }
                        fs::rename(&part_path, &item.path)
                            .await
                            .map_err(|e| CoreError::io(item.path.clone(), e))?;
                        return Ok(());
                    }
                    Err(e) => {
                        let _ = fs::remove_file(&part_path).await;
                        let retriable = is_retriable(&e) && !is_not_found(&e);
                        last_err = Some(e);
                        if retriable && attempt + 1 < MAX_ATTEMPTS {
                            let backoff = BACKOFF_BASE * (1u32 << attempt);
                            tracing::debug!(url = %url, attempt, ?backoff, "download retry");
                            tokio::time::sleep(backoff).await;
                            continue;
                        }
                        break; // 换下一候选源
                    }
                }
            }
        }

        Err(last_err.unwrap_or_else(|| CoreError::Download {
            url: item.url.clone(),
            reason: "no download sources".into(),
        }))
    }

    /// 计算一个下载项的有序候选 URL 列表:主源 + 各 manifest 镜像,每个再经
    /// [`MirrorResolver::candidates`] 展开成(镜像变体 + 官方回退),最后整体去重保序。
    fn candidate_urls(&self, item: &DownloadItem) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for base in std::iter::once(&item.url).chain(item.mirrors.iter()) {
            if base.is_empty() {
                continue;
            }
            for c in self.mirror.candidates(base) {
                if seen.insert(c.clone()) {
                    out.push(c);
                }
            }
        }
        if out.is_empty() {
            out.push(item.url.clone());
        }
        out
    }

    /// 把 `url` 流式写入 `part_path`(不校验、不 rename)。一次成功的字节搬运;
    /// 重试 / 校验 / 落盘由 [`Self::download_one`] 编排。
    async fn fetch_to_part(&self, url: &str, part_path: &PathBuf) -> Result<()> {
        let req = self.apply_cf_auth(self.client.get(url), url);
        let resp = req.send().await?.error_for_status()?;

        let mut file = fs::File::create(part_path)
            .await
            .map_err(|e| CoreError::io(part_path.clone(), e))?;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?; // reqwest::Error -> CoreError::Network
            file.write_all(&chunk)
                .await
                .map_err(|e| CoreError::io(part_path.clone(), e))?;
        }
        // 落盘到操作系统缓冲区即可(后续 rename 会保证可见性);显式 flush 关闭句柄。
        file.flush()
            .await
            .map_err(|e| CoreError::io(part_path.clone(), e))?;
        drop(file);

        Ok(())
    }

    /// 并发下载整批文件,**尽力而为**:逐项独立失败不中止全批,失败项跨趟重投
    /// (排除 404 / 校验不符这类确定性错误)最多 [`MAX_JOB_PASSES`] 趟。
    ///
    /// - 用 `Semaphore` 限制同时在飞的请求数(并发度即 `new` 时设定的值)。
    /// - 可选 `progress` 通道按"已完成项数 / 总项数"上报;每完成一项推送一次。
    /// - 返回 [`DownloadOutcome`],调用方据此决定"必备文件缺失即失败"还是"尽量补"。
    pub async fn download_batch(
        &self,
        items: Vec<DownloadItem>,
        progress: Option<watch::Sender<Progress>>,
    ) -> Result<DownloadOutcome> {
        let total = items.len() as u64;
        let mut outcome = DownloadOutcome {
            total,
            succeeded: 0,
            failed: Vec::new(),
        };

        // 进度以"完成的文件数"为单位(而非字节),对一批小文件更直观且无需估速。
        let stage = "下载文件";
        if let Some(tx) = &progress {
            let _ = tx.send(Progress {
                stage: stage.into(),
                current: 0,
                total,
                speed_bps: 0,
            });
        }
        if total == 0 {
            return Ok(outcome);
        }

        let done = Arc::new(AtomicU64::new(0));
        // 用构造时的并发度而非当前可用许可:共享 Downloader 时别的批次正持有许可,
        // 读 available_permits 会把本批 buffer_unordered 错误地压到很低甚至 0。
        let concurrency = self.concurrency.max(1);
        let mut pending = items;

        for pass in 0..MAX_JOB_PASSES {
            if pending.is_empty() {
                break;
            }
            // 跑完本趟全部 pending 并收集 (item, 结果),不因首个错误中止。
            let results: Vec<(DownloadItem, Result<()>)> =
                futures::stream::iter(pending.into_iter().map(|item| {
                    let this = self.clone();
                    let sem = self.sem.clone();
                    let done = done.clone();
                    let progress = progress.clone();
                    async move {
                        let _permit = sem.acquire().await.expect("semaphore not closed");
                        let r = this.download_one(&item).await;
                        if r.is_ok() {
                            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                            if let Some(tx) = &progress {
                                let _ = tx.send(Progress {
                                    stage: stage.into(),
                                    current: n,
                                    total,
                                    speed_bps: 0,
                                });
                            }
                        }
                        (item, r)
                    }
                }))
                .buffer_unordered(concurrency)
                .collect()
                .await;

            let mut next: Vec<DownloadItem> = Vec::new();
            for (item, r) in results {
                match r {
                    Ok(()) => outcome.succeeded += 1,
                    Err(e) => {
                        // 非确定性错误且还有重投机会 -> 下一趟重试;否则计入失败。
                        if pass + 1 < MAX_JOB_PASSES && !is_permanent(&e) {
                            next.push(item);
                        } else {
                            outcome.failed.push((item, e));
                        }
                    }
                }
            }
            pending = next;
        }

        Ok(outcome)
    }

    /// 并发下载整批文件;**任一项最终失败即返回该错误**(在 [`Self::download_batch`]
    /// 的尽力而为 + 跨趟重投之上,末尾汇总判定)。适用于"必备文件必须全部到位"的场景。
    pub async fn download_all(
        &self,
        items: Vec<DownloadItem>,
        progress: Option<watch::Sender<Progress>>,
    ) -> Result<()> {
        self.download_batch(items, progress).await?.into_result()
    }

    /// GET 原始字节(镜像改写 + 重试)。用于版本 json、清单等小文件。
    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let candidates = self.candidate_urls(&DownloadItem {
            url: url.to_string(),
            ..Default::default()
        });
        let mut last_err: Option<CoreError> = None;
        for candidate in &candidates {
            for attempt in 0..MAX_ATTEMPTS {
                match self
                    .apply_cf_auth(self.client.get(candidate), candidate)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                {
                    Ok(resp) => match resp.bytes().await {
                        Ok(b) => return Ok(b.to_vec()),
                        Err(e) => last_err = Some(CoreError::Network(e)),
                    },
                    Err(e) => last_err = Some(CoreError::Network(e)),
                }
                let retriable = last_err.as_ref().is_some_and(is_retriable)
                    && !last_err.as_ref().is_some_and(is_not_found);
                if retriable && attempt + 1 < MAX_ATTEMPTS {
                    tokio::time::sleep(BACKOFF_BASE * (1u32 << attempt)).await;
                    continue;
                }
                break;
            }
        }
        Err(last_err.unwrap_or_else(|| CoreError::Download {
            url: url.to_string(),
            reason: "no download sources".into(),
        }))
    }

    /// GET 原始字节,但在响应头和流式读取阶段都强制执行最大字节数。
    pub async fn get_bytes_capped(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>> {
        let candidates = self.candidate_urls(&DownloadItem {
            url: url.to_string(),
            ..Default::default()
        });
        let mut last_err: Option<CoreError> = None;
        for candidate in &candidates {
            for attempt in 0..MAX_ATTEMPTS {
                match self
                    .apply_cf_auth(self.client.get(candidate), candidate)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                {
                    Ok(resp) => {
                        if let Some(content_length) = resp.content_length() {
                            if content_length > max_bytes as u64 {
                                last_err = Some(capped_download_error(
                                    candidate,
                                    max_bytes,
                                    content_length,
                                ));
                                break;
                            }
                        }

                        let mut out = Vec::new();
                        let mut stream = resp.bytes_stream();
                        let mut stream_err: Option<CoreError> = None;
                        while let Some(chunk) = stream.next().await {
                            match chunk {
                                Ok(chunk) => {
                                    let next_len = out.len().saturating_add(chunk.len());
                                    if next_len > max_bytes {
                                        stream_err = Some(capped_download_error(
                                            candidate,
                                            max_bytes,
                                            next_len as u64,
                                        ));
                                        break;
                                    }
                                    out.extend_from_slice(&chunk);
                                }
                                Err(e) => {
                                    stream_err = Some(CoreError::Network(e));
                                    break;
                                }
                            }
                        }

                        match stream_err {
                            Some(err) => last_err = Some(err),
                            None => return Ok(out),
                        }
                    }
                    Err(e) => last_err = Some(CoreError::Network(e)),
                }
                let retriable = last_err.as_ref().is_some_and(is_retriable)
                    && !last_err.as_ref().is_some_and(is_not_found)
                    && !last_err.as_ref().is_some_and(is_size_limit);
                if retriable && attempt + 1 < MAX_ATTEMPTS {
                    tokio::time::sleep(BACKOFF_BASE * (1u32 << attempt)).await;
                    continue;
                }
                break;
            }
        }
        Err(last_err.unwrap_or_else(|| CoreError::Download {
            url: url.to_string(),
            reason: "no download sources".into(),
        }))
    }

    /// GET 文本(UTF-8 lossy 由 reqwest::text 处理)。
    pub async fn get_text(&self, url: &str) -> Result<String> {
        let bytes = self.get_bytes(url).await?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// GET 并反序列化为 JSON。反序列化失败映射为 [`CoreError::Parse`],错误信息带上 URL。
    pub async fn get_json<T: DeserializeOwned>(&self, url: &str) -> Result<T> {
        let bytes = self.get_bytes(url).await?;
        serde_json::from_slice(&bytes).map_err(|source| CoreError::Parse {
            what: url.to_string(),
            source,
        })
    }
}

/// 判断一个 host 是否属于 CurseForge(其 CDN 自 2026-07-16 起要求 `x-api-key`)。
/// 命中:`api.curseforge.com`、任何 `*.forgecdn.net`(含 `edge.` / `mediafilez.`)、
/// 任何 `*.curseforge.com`。先 trim + 去尾点 + 转小写归一,再走共享的子域后缀匹配
/// ([`crate::host::host_matches_suffix`])。纯函数,可单测。
pub(crate) fn is_curseforge_host(host: &str) -> bool {
    let h = host.trim().trim_end_matches('.').to_ascii_lowercase();
    crate::host::host_matches_suffix(&h, &["forgecdn.net", "curseforge.com"])
}

/// 从一个 URL 抽出 host 并判断是否为 CurseForge host。无法解析出 host 的 URL 视作非 CF
/// (绝不在不确定时附加 key)。
pub(crate) fn url_is_curseforge(url: &str) -> bool {
    crate::host::host_of(url).map(is_curseforge_host).unwrap_or(false)
}

/// 判断错误是否值得重试。网络层错误(超时、连接重置、DNS、5xx 等)可重试;
/// 校验失败 / 解析失败 / IO 错误一般是确定性问题,重试无意义。
pub(crate) fn is_retriable(err: &CoreError) -> bool {
    match err {
        // reqwest 的 timeout/connect/request/body 错误都可能是瞬时的。
        CoreError::Network(e) => {
            e.is_timeout() || e.is_connect() || e.is_request() || e.is_body() || e.is_status()
        }
        CoreError::Download { .. } => true,
        _ => false,
    }
}

/// 是否为 HTTP 404(资源不存在):该源不该重试,直接换下一候选。
pub(crate) fn is_not_found(err: &CoreError) -> bool {
    matches!(err, CoreError::Network(e) if e.status().map(|s| s.as_u16()) == Some(404))
}

pub(crate) fn capped_download_error(url: &str, max_bytes: usize, actual_bytes: u64) -> CoreError {
    CoreError::Download {
        url: url.to_string(),
        reason: format!(
            "response exceeds maximum size of {max_bytes} bytes: got at least {actual_bytes} bytes"
        ),
    }
}

pub(crate) fn is_size_limit(err: &CoreError) -> bool {
    matches!(
        err,
        CoreError::Download { reason, .. } if reason.contains("exceeds maximum size")
    )
}

/// 是否为"确定性"错误(重投也不会变好):校验不符 / 404。用于决定是否跨趟重投。
pub(crate) fn is_permanent(err: &CoreError) -> bool {
    matches!(err, CoreError::Checksum { .. }) || is_not_found(err)
}
