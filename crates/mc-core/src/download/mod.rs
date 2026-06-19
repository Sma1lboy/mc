//! 并发下载引擎。负责把一批 [`DownloadItem`] 拉到本地磁盘并逐个校验 sha1,是
//! 版本安装 / 资源补全 / 库下载的公共底座。
//!
//! 核心设计:
//! - **全局复用的 `reqwest::Client`**:client 内部维护连接池,跨所有请求共享 keep-alive
//!   连接,避免每次下载都做 TLS 握手。整个 launcher 应只持有一个 [`Downloader`]。
//! - **流式 + 增量哈希**:下载时一边写 `.part` 临时文件一边喂给 sha1,完成后一次校验,
//!   既不把大文件读进内存,也不需要写完再回读算哈希。
//! - **原子落盘**:先写 `<path>.part`,校验通过后 `rename` 到最终路径。rename 在同一
//!   文件系统上是原子的,保证别的进程永远看不到"写了一半 / 哈希错误"的文件。
//! - **幂等跳过**:目标文件已存在且 sha1 匹配则直接跳过,使整批下载可安全重试(断点续整)。
//! - **重试**:瞬时网络错误指数退避重试 3 次;校验失败不重试(更可能是镜像内容本身坏了,
//!   交由上层换源处理)。
//! - **并发**:`Semaphore` 限制同时在飞的请求数,`buffer_unordered` 跑满并发并聚合进度。

pub mod checksum;
pub mod mirror;
pub mod murmur2;

pub use checksum::Checksum;
pub use mirror::MirrorResolver;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde::de::DeserializeOwned;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::{watch, Semaphore};

use mc_types::Progress;

use crate::error::{CoreError, Result};

/// 单个待下载文件的描述。
///
/// - **多源**:`url` 是主源,`mirrors` 是有序的额外候选(如 `.mrpack` 的 `downloads[1..]`)。
///   下载时主源失败会依次回退到镜像改写变体与各候选,任一成功即止。
/// - **多哈希**:`sha1`/`sha512`/`md5` 可选,下载后用"最强可用"的一个强校验
///   (sha512 > sha1 > md5);全缺时只保证下载成功。镜像返回坏文件会触发换源。
#[derive(Debug, Clone, Default)]
pub struct DownloadItem {
    /// 主源 URL(尚未经过镜像改写)。
    pub url: String,
    /// 额外候选源(manifest 提供的镜像),按序在主源之后尝试。
    pub mirrors: Vec<String>,
    /// 目标落盘绝对路径。
    pub path: PathBuf,
    /// 期望 sha1(大小写不敏感十六进制)。
    pub sha1: Option<String>,
    /// 期望 sha512(Modrinth `.mrpack` 以此为准)。
    pub sha512: Option<String>,
    /// 期望 md5(CurseForge / ATLauncher / Technic 常用)。
    pub md5: Option<String>,
    /// 期望大小(字节),仅用于进度展示,不参与校验。
    pub size: Option<u64>,
}

impl DownloadItem {
    /// 便捷构造:单源 + 可选 sha1(覆盖最常见的"版本 json 给 sha1"场景)。
    pub fn new(
        url: impl Into<String>,
        path: PathBuf,
        sha1: Option<String>,
        size: Option<u64>,
    ) -> Self {
        Self { url: url.into(), path, sha1, size, ..Default::default() }
    }

    /// 下载后用于强校验的"最强可用"摘要;全缺返回 `None`(无法校验)。
    pub fn checksum(&self) -> Option<Checksum> {
        Checksum::strongest(self.sha512.as_deref(), self.sha1.as_deref(), self.md5.as_deref())
    }
}

/// 一批下载的结果。`failed` 非空表示有项在所有候选源 + 跨趟重投后仍失败。
#[derive(Debug)]
pub struct DownloadOutcome {
    /// 提交的总项数。
    pub total: u64,
    /// 成功项数。
    pub succeeded: usize,
    /// 最终失败的项及其最后一个错误。
    pub failed: Vec<(DownloadItem, CoreError)>,
}

impl DownloadOutcome {
    /// 是否全部成功。
    pub fn all_ok(&self) -> bool {
        self.failed.is_empty()
    }

    /// 失败项数。
    pub fn failed_count(&self) -> usize {
        self.failed.len()
    }

    /// 折叠为 `Result<()>`:有失败则返回第一个错误,否则 `Ok`。
    pub fn into_result(self) -> Result<()> {
        match self.failed.into_iter().next() {
            None => Ok(()),
            Some((_item, e)) => Err(e),
        }
    }
}

/// 单源内的网络重试次数(首试 + 重试)。校验失败 / 404 不在此重试,直接换源。
const MAX_ATTEMPTS: usize = 3;
/// 整批失败项的最大重投趟数(含首趟)。对齐 Prism `NetJob` 的 3 趟。
const MAX_JOB_PASSES: usize = 3;
/// 指数退避基准:第 n 次失败后等待 `BASE * 2^n`。
const BACKOFF_BASE: Duration = Duration::from_millis(300);

/// 全局复用的并发下载器。克隆开销低(内部 `Arc`),但通常单例使用即可。
#[derive(Clone)]
pub struct Downloader {
    client: reqwest::Client,
    sem: Arc<Semaphore>,
    mirror: MirrorResolver,
}

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
            mirror: MirrorResolver::none(),
        })
    }

    /// 设置镜像改写器(链式)。默认无镜像(直连官方)。
    pub fn with_mirror(mut self, mirror: MirrorResolver) -> Self {
        self.mirror = mirror;
        self
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

        let part_path = part_path(&item.path);
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
        let resp = self.client.get(url).send().await?.error_for_status()?;

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
        let mut outcome = DownloadOutcome { total, succeeded: 0, failed: Vec::new() };

        // 进度以"完成的文件数"为单位(而非字节),对一批小文件更直观且无需估速。
        let stage = "下载文件";
        if let Some(tx) = &progress {
            let _ = tx.send(Progress { stage: stage.into(), current: 0, total, speed_bps: 0 });
        }
        if total == 0 {
            return Ok(outcome);
        }

        let done = Arc::new(AtomicU64::new(0));
        let concurrency = self.sem.available_permits().max(1);
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
        let url = self.mirror.rewrite(url);
        let mut last_err: Option<CoreError> = None;
        for attempt in 0..MAX_ATTEMPTS {
            match self
                .client
                .get(&url)
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
            if attempt + 1 < MAX_ATTEMPTS && last_err.as_ref().is_some_and(is_retriable) {
                tokio::time::sleep(BACKOFF_BASE * (1u32 << attempt)).await;
                continue;
            }
            break;
        }
        Err(last_err.unwrap_or_else(|| CoreError::Download {
            url,
            reason: "exhausted retries".into(),
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

/// 计算 `<path>.part` 临时文件路径(在原扩展名后追加 `.part`)。
fn part_path(path: &PathBuf) -> PathBuf {
    let mut s = path.clone().into_os_string();
    s.push(".part");
    PathBuf::from(s)
}

/// 判断错误是否值得重试。网络层错误(超时、连接重置、DNS、5xx 等)可重试;
/// 校验失败 / 解析失败 / IO 错误一般是确定性问题,重试无意义。
fn is_retriable(err: &CoreError) -> bool {
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
fn is_not_found(err: &CoreError) -> bool {
    matches!(err, CoreError::Network(e) if e.status().map(|s| s.as_u16()) == Some(404))
}

/// 是否为"确定性"错误(重投也不会变好):校验不符 / 404。用于决定是否跨趟重投。
fn is_permanent(err: &CoreError) -> bool {
    matches!(err, CoreError::Checksum { .. }) || is_not_found(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_path_appends_suffix() {
        let p = PathBuf::from("/tmp/foo/client.jar");
        assert_eq!(part_path(&p), PathBuf::from("/tmp/foo/client.jar.part"));
    }

    #[test]
    fn new_uses_mirror_none_by_default() {
        let d = Downloader::new(4).unwrap();
        // 默认无镜像:URL 原样返回。
        assert_eq!(
            d.mirror.rewrite("https://libraries.minecraft.net/a/b.jar"),
            "https://libraries.minecraft.net/a/b.jar"
        );
    }

    #[test]
    fn with_mirror_applies_rewrite() {
        let d = Downloader::new(4).unwrap().with_mirror(MirrorResolver::bmclapi());
        assert_eq!(
            d.mirror.rewrite("https://libraries.minecraft.net/a/b.jar"),
            "https://bmclapi2.bangbang93.com/maven/a/b.jar"
        );
    }

    #[test]
    fn zero_concurrency_is_bumped_to_one() {
        // Semaphore(0) 会永久阻塞;new 必须把 0 提升到 1。
        let d = Downloader::new(0).unwrap();
        assert!(d.sem.available_permits() >= 1);
    }
}
