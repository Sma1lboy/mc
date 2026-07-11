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
        Self {
            url: url.into(),
            path,
            sha1,
            size,
            ..Default::default()
        }
    }

    /// 下载后用于强校验的"最强可用"摘要;全缺返回 `None`(无法校验)。
    pub fn checksum(&self) -> Option<Checksum> {
        Checksum::strongest(
            self.sha512.as_deref(),
            self.sha1.as_deref(),
            self.md5.as_deref(),
        )
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
    /// 构造时设定的并发度。`sem` 的可用许可会随在飞请求消耗,故并发度单独存档,
    /// 避免与他人共享同一 Downloader 时被错误地降速(见 download_batch)。
    concurrency: usize,
    mirror: MirrorResolver,
    /// 可选的 CurseForge API key。仅在下载 CurseForge 自家 host(api.curseforge.com /
    /// *.forgecdn.net)时随 `x-api-key` 头发送(2026-07-16 起 CDN 要求鉴权);其它 host
    /// 一律不带。**secret,勿打日志。**
    cf_api_key: Option<String>,
}

mod downloader;
#[cfg(test)]
mod tests;
