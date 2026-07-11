//! Modrinth API v2 后端(开放、无需 API key)。
//!
//! 文档:<https://docs.modrinth.com/api/>。这里只用到三个只读端点:
//! - `GET /v2/search`            —— 搜索项目,facets 过滤 project_type/版本/loader
//! - `GET /v2/project/{id}/version` —— 列项目版本,可按 loaders/game_versions 过滤
//! - `GET /v2/project/{id}`      —— 取单个项目元信息
//!
//! 设计要点:
//! - **自带 `reqwest::Client`**。Modrinth 要求带一个能联系到作者的 User-Agent
//!   (否则可能限流/封禁),所以我们硬编码一个含仓库地址的 UA。
//! - 平台原始 json 的字段名(`project_type` / `version_number` / `game_versions`
//!   等)与统一模型不同,故这里用一组 `Raw*` 内部类型承接原始 json,再由纯映射
//!   函数转成 [`crate::modplatform`] 的统一模型。映射函数无 IO、可单测。
//! - 容错:缺字段一律走 `#[serde(default)]` 给默认值,不让单个字段缺失把整次请
//!   求打挂。HTTP/网络错误映射成 [`CoreError::Network`],反序列化错误映射成
//!   [`CoreError::Parse`]。

use serde::Deserialize;

use crate::error::{CoreError, Result};

use super::{
    Dependency, ProjectSideSupport, ProjectVersion, ResourceKind, SearchHit, SearchQuery,
    SortMethod, VersionFile,
};

/// Modrinth API v2 根地址。
const API_BASE: &str = "https://api.modrinth.com/v2";

/// Modrinth 要求的 User-Agent(含联系方式形式)。
const USER_AGENT: &str = "mc-launcher/0.1 (github.com/sma1lboy/mc-launcher)";

/// Modrinth 后端客户端。`new()` 自带一个配置好 UA 的 `reqwest::Client`。
#[derive(Debug, Clone)]
pub struct ModrinthApi {
    client: reqwest::Client,
    base: String,
}

impl Default for ModrinthApi {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程级共享的 Modrinth `reqwest::Client`。`reqwest::Client` 内部是 `Arc`,克隆共享同一
/// TLS 配置与连接池;每次 [`ModrinthApi::new`] 复用它,而非重建一个新池(否则每次请求都付
/// 冷连接代价)。配置与旧的 per-call 构造**完全一致**(仅固化 UA),对所有调用方行为不变。
///
/// 失败(仅 TLS 后端初始化失败)走 `expect`——属环境级灾难,失败即代表整个进程无法发请求。
fn shared_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .expect("failed to build reqwest client for Modrinth")
        })
        .clone()
}

mod api;
mod cache;
mod dto;
mod facets;
mod provider;
#[cfg(test)]
mod tests;

pub use dto::*;
pub use facets::*;
pub use provider::*;
use cache::*;
pub(crate) use api::*;
