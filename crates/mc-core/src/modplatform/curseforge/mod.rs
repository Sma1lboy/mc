//! CurseForge "Flame" API v1 后端(`api.curseforge.com`,**需要** `x-api-key`)。
//!
//! 文档:<https://docs.curseforge.com/>(`gameId=432` = Minecraft)。本模块用到这些端点:
//! - `GET  /v1/mods/search`     —— 搜索项目(`searchFilter` / `classId` / `gameVersion` / `modLoaderType`)
//! - `POST /v1/mods`            —— 批量取项目元信息(`{"modIds":[...]}`)
//! - `GET  /v1/mods/{id}/files` —— 列某项目的文件(= 我们的"版本")
//! - `POST /v1/mods/files`      —— 批量按 fileId 取文件(`{"fileIds":[...]}`),整合包导入把
//!   manifest 里的 (projectID,fileID) 变成真实下载 URL 的命脉
//! - `POST /v1/fingerprints`    —— murmur2 指纹反查(`{"fingerprints":[...]}`),整合包导出/去重用
//!
//! 设计要点(对齐 [`super::modrinth`]):
//! - **自带 `reqwest::Client`**,默认头里固化 `x-api-key` + 一个含仓库地址的 User-Agent。
//! - 平台原始 json(camelCase、整数枚举哈希算法、`gameVersions` 是混杂扁平数组)与统一
//!   模型差异很大,故用一组 `Raw*`/`FlameApi*` 内部类型承接原始 json,再由**纯映射函数**
//!   转成 [`crate::modplatform`] 的统一模型。映射函数无 IO、可单测。
//! - 容错:每个 `Raw` 字段一律 `#[serde(default)]`,缺字段不让整次请求打挂。HTTP/网络错误
//!   映射成 [`CoreError::Network`],反序列化错误映射成 [`CoreError::Parse`]。
//!
//! API key 是 **secret**:从环境变量 `MC_CF_API_KEY` 读取(镜像项目 `MC_MSA_CLIENT_ID`
//! 的约定),**绝不**硬编码、勿入库、勿打日志。详见 [`FlameApi::from_env`]。
//!
//! ## 几个 CurseForge 专有易错点
//! - **`downloadUrl` 可空 = BLOCKED**(作者禁第三方分发):映射后 [`VersionFile::url`] 为
//!   空串,调用方据"url 为空"识别 blocked,绝不猜 URL。
//! - **murmur2 不是标准 murmur2**:seed=1、先滤掉字节 9/10/13/32(tab/LF/CR/空格)再算。
//!   指纹由 [`crate::download::murmur2`] 计算;本模块只负责把已算好的 u32 提交反查。
//! - **`hashes[].algo` 是整数**(1=sha1,2=md5),不是字符串;CF 只保 sha1。
//! - **`gameVersions[]` 是扁平异构数组**(MC 版本 + loader 名 + Client/Server),客户端切分:
//!   含 `.` 的当游戏版本,匹配 forge/fabric/neoforge/quilt 的当 loader。
//! - **`/mods/files` 单 id 偶发返回对象而非数组**:`data` 用容忍数组或单对象的反序列化处理。

use serde::Deserialize;

use crate::error::{CoreError, Result};

use super::{
    HashAlgo, ProjectSideSupport, ProjectVersion, ProviderCaps, ProviderId, ResolvedFile,
    ResourceKind, SearchHit, SearchQuery, SortMethod, VersionFile,
};

/// CurseForge Flame API v1 根地址。
const API_BASE: &str = "https://api.curseforge.com/v1";

/// Minecraft 的 CurseForge `gameId`。
const GAME_ID: i64 = 432;

/// 读取 API key 的环境变量名(secret,见模块文档)。
const API_KEY_ENV: &str = "MC_CF_API_KEY";

/// User-Agent(含联系方式形式,和 Modrinth 后端一致)。
const USER_AGENT: &str = "mc-launcher/0.1 (github.com/sma1lboy/mc-launcher)";

/// CurseForge `classId`:Minecraft 各资源大类。`/mods/search` 用它锁定资源类型。
const CLASS_MOD: i64 = 6;
const CLASS_MODPACK: i64 = 4471;
const CLASS_RESOURCEPACK: i64 = 12;
const CLASS_SHADERPACK: i64 = 6552;

/// CurseForge `modLoaderType` 枚举值(`/mods/search` 的 `modLoaderType` 参数)。
/// 0=Any 1=Forge 2=Cauldron 3=LiteLoader 4=Fabric 5=Quilt 6=NeoForge。
fn loader_type_id(loader: &str) -> Option<i64> {
    match loader.to_ascii_lowercase().as_str() {
        "forge" => Some(1),
        "fabric" => Some(4),
        "quilt" => Some(5),
        "neoforge" => Some(6),
        _ => None,
    }
}

/// CurseForge `sortField`(`/mods/search` 的 `sortField` 参数)。
/// 1=Featured 2=Popularity 3=LastUpdated 4=Name 5=Author 6=TotalDownloads 7=Category 8=GameVersion。
fn sort_field_id(sort: SortMethod) -> i64 {
    match sort {
        // 没有真正的"相关度",CF 默认用 Popularity 作为最贴近的排序。
        SortMethod::Relevance => 2,
        SortMethod::Downloads => 6,
        // CF 没有"按发布时间",最接近的是 LastUpdated。
        SortMethod::Newest => 3,
        SortMethod::Updated => 3,
    }
}

/// 把统一 [`ResourceKind`] 映射到 CurseForge `classId`。数据包在 CF 没有独立 class,
/// 与 mod 同 class(用 category 区分),故回退到 [`CLASS_MOD`]。
fn class_id(kind: ResourceKind) -> i64 {
    match kind {
        ResourceKind::Mod => CLASS_MOD,
        ResourceKind::Modpack => CLASS_MODPACK,
        ResourceKind::ResourcePack => CLASS_RESOURCEPACK,
        ResourceKind::Shader => CLASS_SHADERPACK,
        ResourceKind::Datapack => CLASS_MOD,
    }
}

/// CurseForge Flame API 客户端。`new()` 自带配置好 `x-api-key` + UA 的 `reqwest::Client`。
#[derive(Debug, Clone)]
pub struct FlameApi {
    client: reqwest::Client,
    base: String,
    api_key: String,
}

/// 实际构造一个配好 `x-api-key` + UA 默认头的 `reqwest::Client`(逻辑同旧 `new`,抽出来供
/// [`shared_client`] 缓存)。`x-api-key` 因 key 而异,故无法做成全进程唯一的客户端,只能按
/// key 缓存。
fn build_client(api_key: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    // key 来自 env,理论上可能含非法 header 字节;此时退化为不带默认头(请求会 401),
    // 但不让一个坏 key 把整个进程 panic。
    if let Ok(mut value) = reqwest::header::HeaderValue::from_str(api_key) {
        value.set_sensitive(true);
        headers.insert("x-api-key", value);
    }

    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .default_headers(headers)
        // reqwest 仅在 TLS 后端初始化失败时报错,属于环境级灾难;静态配置失败即代表
        // 整个进程无法发任何请求,直接 expect 暴露问题。
        .build()
        .expect("failed to build reqwest client for CurseForge")
}

/// 进程级、**按 key 缓存**的 CurseForge `reqwest::Client`。`x-api-key` 固化进默认头,故不同
/// key 必须用不同客户端;同一 key 在一次进程内复用同一连接池(克隆共享),免去每次
/// [`FlameApi::new`] 重建连接池的冷连接代价。配置与旧 per-call 构造逐字节一致,行为不变。
fn shared_client(api_key: &str) -> reqwest::Client {
    static CLIENTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, reqwest::Client>>,
    > = std::sync::OnceLock::new();
    let clients = CLIENTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = clients.lock().expect("CurseForge client cache poisoned");
    if let Some(client) = guard.get(api_key) {
        return client.clone();
    }
    let client = build_client(api_key);
    guard.insert(api_key.to_string(), client.clone());
    client
}

mod api;
mod dto;
mod provider;
#[cfg(test)]
mod tests;

pub use dto::*;
pub use provider::*;
