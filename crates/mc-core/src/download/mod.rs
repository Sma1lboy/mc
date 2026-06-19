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

pub use mirror::MirrorResolver;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::{watch, Semaphore};

use mc_types::Progress;

use crate::error::{CoreError, Result};

/// 单个待下载文件的描述。`sha1`/`size` 来自版本 json,可选;提供 sha1 时下载后会强制校验。
#[derive(Debug, Clone)]
pub struct DownloadItem {
    /// 源 URL(尚未经过镜像改写)。
    pub url: String,
    /// 目标落盘绝对路径。
    pub path: PathBuf,
    /// 期望的 sha1(小写或大写十六进制)。`None` 表示无法校验,只保证下载成功。
    pub sha1: Option<String>,
    /// 期望大小(字节),仅用于进度展示,不参与校验。
    pub size: Option<u64>,
}

/// 网络重试次数(首试 + 重试)。校验失败不计入此处。
const MAX_ATTEMPTS: usize = 3;
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

    /// 下载单个文件:镜像改写 -> 幂等跳过 -> 流式下载 + 增量 sha1 -> 校验 -> 原子落盘。
    pub async fn download_one(&self, item: &DownloadItem) -> Result<()> {
        // 1) 已存在且 sha1 匹配则直接跳过,使重试整批时不重复下载已完成项。
        if let Some(expected) = &item.sha1 {
            if checksum::verify_sha1(&item.path, expected) {
                return Ok(());
            }
        }

        let url = self.mirror.rewrite(&item.url);

        // 2) 确保父目录存在。
        if let Some(parent) = item.path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|e| CoreError::io(parent.to_path_buf(), e))?;
        }

        let part_path = part_path(&item.path);

        // 3) 带重试地执行下载到 .part 并返回算出的 sha1。
        let mut last_err: Option<CoreError> = None;
        for attempt in 0..MAX_ATTEMPTS {
            match self.fetch_to_part(&url, &part_path).await {
                Ok(actual_sha1) => {
                    // 4) 校验 sha1(如提供)。不匹配 -> 删 .part,返回 Checksum 错误(不重试)。
                    if let Some(expected) = &item.sha1 {
                        if !actual_sha1.eq_ignore_ascii_case(expected.trim()) {
                            let _ = fs::remove_file(&part_path).await;
                            return Err(CoreError::Checksum {
                                path: item.path.clone(),
                                expected: expected.clone(),
                                actual: actual_sha1,
                            });
                        }
                    }
                    // 5) 原子 rename 到最终路径。
                    fs::rename(&part_path, &item.path)
                        .await
                        .map_err(|e| CoreError::io(item.path.clone(), e))?;
                    return Ok(());
                }
                Err(e) => {
                    // 网络/IO 错误:清理半成品后指数退避重试。
                    let _ = fs::remove_file(&part_path).await;
                    let retriable = is_retriable(&e);
                    last_err = Some(e);
                    if retriable && attempt + 1 < MAX_ATTEMPTS {
                        let backoff = BACKOFF_BASE * (1u32 << attempt);
                        tracing::debug!(url = %url, attempt, ?backoff, "download retry");
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    break;
                }
            }
        }

        Err(last_err.unwrap_or_else(|| CoreError::Download {
            url: url.clone(),
            reason: "exhausted retries".into(),
        }))
    }

    /// 把 `url` 流式写入 `part_path`,同时增量计算并返回 sha1。
    ///
    /// 不做重试、不做校验、不做 rename —— 这些由 [`Self::download_one`] 编排,
    /// 本函数只负责"一次成功的字节搬运"。
    async fn fetch_to_part(&self, url: &str, part_path: &PathBuf) -> Result<String> {
        let resp = self.client.get(url).send().await?.error_for_status()?;

        let mut file = fs::File::create(part_path)
            .await
            .map_err(|e| CoreError::io(part_path.clone(), e))?;

        let mut hasher = Sha1::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?; // reqwest::Error -> CoreError::Network
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|e| CoreError::io(part_path.clone(), e))?;
        }
        // 落盘到操作系统缓冲区即可(后续 rename 会保证可见性);显式 flush 关闭句柄。
        file.flush()
            .await
            .map_err(|e| CoreError::io(part_path.clone(), e))?;
        drop(file);

        Ok(hex::encode(hasher.finalize()))
    }

    /// 并发下载整批文件。
    ///
    /// - 用 `Semaphore` 限制同时在飞的请求数(并发度即 `new` 时设定的值)。
    /// - 可选 `progress` 通道按"已完成项数 / 总项数"上报;每完成一项推送一次。
    /// - 任一项失败立即返回第一个错误(其余 in-flight 任务随 stream drop 自然取消)。
    pub async fn download_all(
        &self,
        items: Vec<DownloadItem>,
        progress: Option<watch::Sender<Progress>>,
    ) -> Result<()> {
        let total = items.len() as u64;
        if total == 0 {
            return Ok(());
        }

        // 进度以"完成的文件数"为单位(而非字节),对一批小文件更直观且无需估速。
        let done = Arc::new(AtomicU64::new(0));
        let stage = "下载文件";
        if let Some(tx) = &progress {
            let _ = tx.send(Progress { stage: stage.into(), current: 0, total, speed_bps: 0 });
        }

        let concurrency = self.sem.available_permits().max(1);

        // 为每个 item 生成一个受信号量约束的下载 future,用 buffer_unordered 跑满并发。
        let mut stream = futures::stream::iter(items.into_iter().map(|item| {
            let this = self.clone();
            let sem = self.sem.clone();
            let done = done.clone();
            let progress = progress.clone();
            async move {
                // acquire 失败仅在信号量被关闭时发生,这里永不关闭,unwrap 安全。
                let _permit = sem.acquire().await.expect("semaphore not closed");
                this.download_one(&item).await?;
                // 完成计数自增并上报。Relaxed 足够:我们只需要单调递增的近似进度。
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if let Some(tx) = &progress {
                    let _ = tx.send(Progress {
                        stage: stage.into(),
                        current: n,
                        total,
                        speed_bps: 0,
                    });
                }
                Ok::<(), CoreError>(())
            }
        }))
        .buffer_unordered(concurrency);

        // 收集结果:遇到第一个错误立即返回,提前结束整批。
        while let Some(res) = stream.next().await {
            res?;
        }
        Ok(())
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
