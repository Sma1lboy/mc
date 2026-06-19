//! 元数据层(meta):负责从 Mojang 的远端服务拉取并解析"清单类"数据,
//! 并把这些数据翻译成 download 模块可直接消费的 [`DownloadItem`] 列表。
//!
//! 本模块刻意分成两类函数:
//! - `async fetch_*`:有 IO,通过 [`Downloader`] 发请求并反序列化;
//! - `pure fn *_items`:无 IO 的纯映射,输入已解析好的结构 + [`GamePaths`],
//!   输出下载任务。纯函数便于单元测试,也让上层(instance/安装流程)可以
//!   先把所有下载项收集起来统一交给 [`Downloader::download_all`]。
//!
//! 设计要点:版本 json 的"原文"在 [`fetch_version_json`] 中以 `String` 形态返回,
//! 由 instance 层落盘到 `versions/<id>/<id>.json`,从而保留 Mojang/loader 的
//! 原始字节(避免重新序列化丢失字段)。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use mc_types::{ManifestVersion, ReleaseKind};

use crate::download::{DownloadItem, Downloader};
use crate::error::Result;
use crate::paths::GamePaths;
use crate::version::{
    AssetIndexRef, LaunchProfile, RuntimeContext, DEFAULT_LIBRARIES_MAVEN,
};

/// Mojang 官方版本清单(manifest v2,带每个版本 json 的 sha1)。
pub const VERSION_MANIFEST_URL: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// 资源对象的下载根。资源是内容寻址的:`<host>/<hash[0..2]>/<hash>`。
const RESOURCES_BASE_URL: &str = "https://resources.download.minecraft.net";

// ---------------------------------------------------------------------------
// 版本清单 (version manifest)
// ---------------------------------------------------------------------------

/// 清单中 `versions` 数组的单个原始条目。字段名严格对齐 Mojang manifest v2。
///
/// 单独用一个内部 DTO 而不是直接反序列化成 [`ManifestVersion`],是因为
/// manifest 里的 `type` 是字符串(release/snapshot/old_beta/old_alpha),
/// 我们要把它映射成强类型 [`ReleaseKind`];同时 `releaseTime` 是 camelCase,
/// 需要 rename。保持 DTO 与领域类型分离,远端字段变动不会污染 mc-types。
#[derive(Debug, Clone, Deserialize)]
struct RawManifestEntry {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(default)]
    sha1: String,
    #[serde(default, rename = "releaseTime")]
    release_time: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawManifest {
    #[serde(default)]
    versions: Vec<RawManifestEntry>,
}

/// 把 manifest 的字符串 `type` 映射成 [`ReleaseKind`]。
///
/// 未知类型被宽容地当作 `Release` 处理——Mojang 偶尔会引入新的快照衍生类型,
/// 让整份清单因为一个陌生字符串而解析失败并不划算;调用方拿到的列表只是用于
/// 展示/选择,Release 是最安全的兜底。
fn parse_release_kind(s: &str) -> ReleaseKind {
    match s {
        "release" => ReleaseKind::Release,
        "snapshot" => ReleaseKind::Snapshot,
        "old_beta" => ReleaseKind::OldBeta,
        "old_alpha" => ReleaseKind::OldAlpha,
        _ => ReleaseKind::Release,
    }
}

/// 把原始清单条目映射成领域类型 [`ManifestVersion`]。纯函数,便于测试。
fn map_manifest(raw: RawManifest) -> Vec<ManifestVersion> {
    raw.versions
        .into_iter()
        .map(|e| ManifestVersion {
            id: e.id,
            kind: parse_release_kind(&e.kind),
            url: e.url,
            sha1: e.sha1,
            release_time: e.release_time,
        })
        .collect()
}

/// 拉取并解析 Mojang 版本清单,返回所有可安装版本(顺序保持清单原序,
/// 即最新版本在前)。
pub async fn fetch_manifest(dl: &Downloader) -> Result<Vec<ManifestVersion>> {
    let raw: RawManifest = dl.get_json(VERSION_MANIFEST_URL).await?;
    Ok(map_manifest(raw))
}

/// 拉取某个版本的 version json **原文字符串**。
///
/// 返回未经任何二次序列化的原始文本,交由 instance 层落盘,以完整保留
/// Mojang/loader 的所有字段(包括我们暂未建模的字段)。
pub async fn fetch_version_json(dl: &Downloader, entry: &ManifestVersion) -> Result<String> {
    dl.get_text(&entry.url).await
}

// ---------------------------------------------------------------------------
// 资源索引 (asset index)
// ---------------------------------------------------------------------------

/// 单个资源对象的元数据。`hash` 同时充当内容寻址的文件名与校验值。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

/// 资源索引 json:逻辑路径 → 资源对象。
///
/// 用 [`BTreeMap`] 保证遍历顺序稳定(对生成的下载列表顺序可预测,利于测试与
/// 进度展示),同时与 Mojang 的索引结构一一对应。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetIndexJson {
    pub objects: BTreeMap<String, AssetObject>,
}

/// 按 [`AssetIndexRef::url`] 拉取并解析资源索引。
pub async fn fetch_asset_index(dl: &Downloader, idx: &AssetIndexRef) -> Result<AssetIndexJson> {
    dl.get_json(&idx.url).await
}

/// 把资源索引展开成下载任务列表。
///
/// 每个对象:
/// - url  = `<resources>/<hash[0..2]>/<hash>`
/// - path = `assets/objects/<hash[0..2]>/<hash>`(由 [`GamePaths::asset_object`] 给出)
/// - sha1 = 对象 hash(资源即内容寻址,hash 就是其 sha1)
/// - size = 索引声明的大小
///
/// 纯函数:不触网、不读盘,只做映射,便于单元测试。
pub fn asset_download_items(index: &AssetIndexJson, paths: &GamePaths) -> Vec<DownloadItem> {
    index
        .objects
        .values()
        .map(|obj| {
            let prefix = &obj.hash[0..2.min(obj.hash.len())];
            DownloadItem {
                url: format!("{RESOURCES_BASE_URL}/{prefix}/{}", obj.hash),
                path: paths.asset_object(&obj.hash),
                sha1: Some(obj.hash.clone()),
                size: Some(obj.size),
                ..Default::default()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 库 / 客户端 jar
// ---------------------------------------------------------------------------

/// 把一个已解析的 [`LaunchProfile`] 里所有"在当前平台生效"的库展开成下载任务。
///
/// 对每个库:
/// - 先用 [`Library::applies`] 过滤掉不适用于当前平台/特性的条目;
/// - 取主 classpath jar([`Library::classpath_file`],基准 maven 为
///   [`DEFAULT_LIBRARIES_MAVEN`]);
/// - 若该库还带本地原生库(natives),再取 [`Library::native_file`]。
///
/// 每个 [`crate::version::ResolvedFile`] 的 `path` 是相对 `libraries/` 的仓库路径,
/// 因此落盘路径 = `libraries_dir().join(resolved.path)`。
///
/// 纯函数:所有 IO 都被前移到了 version/library 层的解析里,这里只做路径拼接。
pub fn library_download_items(
    profile: &LaunchProfile,
    paths: &GamePaths,
    ctx: &RuntimeContext,
) -> Vec<DownloadItem> {
    let libraries_dir = paths.libraries_dir();
    let mut items = Vec::new();

    // Classpath jars (main artifacts; excludes native bundles).
    for lib in crate::version::classpath_libraries(&profile.libraries, ctx) {
        if let Some(resolved) = lib.classpath_file(DEFAULT_LIBRARIES_MAVEN) {
            items.push(DownloadItem {
                url: resolved.url,
                path: libraries_dir.join(&resolved.path),
                sha1: resolved.sha1,
                size: resolved.size,
                ..Default::default()
            });
        }
        // Old-style natives (pre-1.19) carried by a regular library via its map.
        if let Some(native) = lib.native_file(ctx) {
            items.push(DownloadItem {
                url: native.url,
                path: libraries_dir.join(&native.path),
                sha1: native.sha1,
                size: native.size,
                ..Default::default()
            });
        }
    }

    // New-style native bundles (1.19+), best arch match per artifact.
    for lib in crate::version::select_native_libraries(&profile.libraries, ctx) {
        if let Some(resolved) = lib.classpath_file(DEFAULT_LIBRARIES_MAVEN) {
            items.push(DownloadItem {
                url: resolved.url,
                path: libraries_dir.join(&resolved.path),
                sha1: resolved.sha1,
                size: resolved.size,
                ..Default::default()
            });
        }
    }

    items
}

/// 生成客户端主 jar(`versions/<id>/<id>.jar`)的下载任务。
///
/// 仅当 profile 里带有 `client_download` 时才返回;否则 `None`(例如某些纯 loader
/// profile 未声明客户端下载,会从其继承链的 vanilla 父级取得)。
pub fn client_jar_item(profile: &LaunchProfile, paths: &GamePaths) -> Option<DownloadItem> {
    let art = profile.client_download.as_ref()?;
    Some(DownloadItem {
        url: art.url.clone(),
        path: paths.version_jar(&profile.id),
        sha1: art.sha1.clone(),
        size: art.size,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::VersionJson;

    // ---- manifest 映射 ----

    #[test]
    fn maps_manifest_types() {
        let raw: RawManifest = serde_json::from_str(
            r#"{"versions":[
                {"id":"1.20.1","type":"release","url":"https://x/1.json","sha1":"aaa","releaseTime":"2023-06-12T00:00:00+00:00"},
                {"id":"23w14a","type":"snapshot","url":"https://x/2.json","sha1":"bbb","releaseTime":"2023-04-05T00:00:00+00:00"},
                {"id":"b1.7.3","type":"old_beta","url":"https://x/3.json"},
                {"id":"a1.0.4","type":"old_alpha","url":"https://x/4.json"},
                {"id":"weird","type":"future_kind","url":"https://x/5.json"}
            ]}"#,
        )
        .unwrap();
        let versions = map_manifest(raw);
        assert_eq!(versions.len(), 5);
        assert_eq!(versions[0].kind, ReleaseKind::Release);
        assert_eq!(versions[0].id, "1.20.1");
        assert_eq!(versions[0].sha1, "aaa");
        assert_eq!(versions[1].kind, ReleaseKind::Snapshot);
        assert_eq!(versions[2].kind, ReleaseKind::OldBeta);
        assert_eq!(versions[3].kind, ReleaseKind::OldAlpha);
        // 未知类型兜底为 Release。
        assert_eq!(versions[4].kind, ReleaseKind::Release);
        // 缺省字段(sha1/releaseTime)被 serde default 成空串。
        assert_eq!(versions[2].sha1, "");
        assert_eq!(versions[2].release_time, "");
    }

    // ---- 资源索引展开 ----

    fn sample_index() -> AssetIndexJson {
        // 两个对象,hash 前两位不同,便于验证分桶路径与 url。
        serde_json::from_str::<AssetIndexJson>(
            r#"{"objects":{
                "minecraft/sounds/a.ogg":{"hash":"ab12cd34ef","size":111},
                "icons/icon.png":{"hash":"00ffeedd","size":222}
            }}"#,
        )
        .unwrap()
    }

    #[test]
    fn asset_items_paths_and_urls() {
        let index = sample_index();
        let paths = GamePaths::new("/games/mc");
        let mut items = asset_download_items(&index, &paths);
        // BTreeMap 按 key 排序:"icons/..." < "minecraft/..."。
        items.sort_by(|a, b| a.url.cmp(&b.url));

        // 找到 hash 为 00ffeedd 的那个对象进行精确断言。
        let zero = items.iter().find(|i| i.sha1.as_deref() == Some("00ffeedd")).unwrap();
        assert_eq!(
            zero.url,
            "https://resources.download.minecraft.net/00/00ffeedd"
        );
        assert_eq!(
            zero.path,
            std::path::PathBuf::from("/games/mc/assets/objects/00/00ffeedd")
        );
        assert_eq!(zero.size, Some(222));

        let other = items.iter().find(|i| i.sha1.as_deref() == Some("ab12cd34ef")).unwrap();
        assert_eq!(
            other.url,
            "https://resources.download.minecraft.net/ab/ab12cd34ef"
        );
        assert_eq!(
            other.path,
            std::path::PathBuf::from("/games/mc/assets/objects/ab/ab12cd34ef")
        );
        assert_eq!(items.len(), 2);
    }

    // ---- 库展开 ----

    fn profile_from(version_json: &str) -> LaunchProfile {
        let vj: VersionJson = VersionJson::parse(version_json).unwrap();
        LaunchProfile::from_chain(&[vj])
    }

    #[test]
    fn library_items_use_explicit_artifact() {
        // 带显式 downloads.artifact:url/path/sha1/size 全部应被透传。
        let profile = profile_from(
            r#"{
                "id":"1.20.1",
                "libraries":[
                    {"name":"com.example:foo:1.2.3",
                     "downloads":{"artifact":{
                        "path":"com/example/foo/1.2.3/foo-1.2.3.jar",
                        "url":"https://cdn/foo.jar",
                        "sha1":"deadbeef",
                        "size":42}}}
                ]
            }"#,
        );
        let paths = GamePaths::new("/games/mc");
        let ctx = RuntimeContext::default();
        let items = library_download_items(&profile, &paths, &ctx);

        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(it.url, "https://cdn/foo.jar");
        assert_eq!(
            it.path,
            std::path::PathBuf::from("/games/mc/libraries/com/example/foo/1.2.3/foo-1.2.3.jar")
        );
        assert_eq!(it.sha1.as_deref(), Some("deadbeef"));
        assert_eq!(it.size, Some(42));
    }

    #[test]
    fn library_items_synthesize_url_only_maven() {
        // 仅有坐标 + 自定义 maven base(Forge/Fabric 风格):url 由坐标拼出,
        // path 为坐标对应的仓库相对路径。
        let profile = profile_from(
            r#"{
                "id":"x",
                "libraries":[
                    {"name":"net.fabricmc:fabric-loader:0.15.0",
                     "url":"https://maven.fabricmc.net/"}
                ]
            }"#,
        );
        let paths = GamePaths::new("/games/mc");
        let ctx = RuntimeContext::default();
        let items = library_download_items(&profile, &paths, &ctx);

        assert_eq!(items.len(), 1);
        let it = &items[0];
        assert_eq!(
            it.url,
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.15.0/fabric-loader-0.15.0.jar"
        );
        assert_eq!(
            it.path,
            std::path::PathBuf::from(
                "/games/mc/libraries/net/fabricmc/fabric-loader/0.15.0/fabric-loader-0.15.0.jar"
            )
        );
        // url-only 库没有 sha1/size。
        assert!(it.sha1.is_none());
        assert!(it.size.is_none());
    }

    #[test]
    fn library_items_skip_non_applicable() {
        // 规则 disallow 当前平台之外不可能命中的库,这里用一条 action:"disallow"
        // 的空规则使该库永远不生效,验证被跳过。
        let profile = profile_from(
            r#"{
                "id":"x",
                "libraries":[
                    {"name":"com.example:only-nowhere:1.0",
                     "rules":[{"action":"disallow"}],
                     "downloads":{"artifact":{"url":"https://cdn/nope.jar"}}}
                ]
            }"#,
        );
        let paths = GamePaths::new("/games/mc");
        let ctx = RuntimeContext::default();
        let items = library_download_items(&profile, &paths, &ctx);
        assert!(items.is_empty(), "disallowed library must be skipped");
    }

    // ---- 客户端 jar ----

    #[test]
    fn client_jar_item_built_from_download() {
        let profile = profile_from(
            r#"{
                "id":"1.20.1",
                "libraries":[],
                "downloads":{"client":{
                    "url":"https://cdn/client.jar",
                    "sha1":"cafef00d",
                    "size":1000}}
            }"#,
        );
        let paths = GamePaths::new("/games/mc");
        let item = client_jar_item(&profile, &paths).unwrap();
        assert_eq!(item.url, "https://cdn/client.jar");
        assert_eq!(
            item.path,
            std::path::PathBuf::from("/games/mc/versions/1.20.1/1.20.1.jar")
        );
        assert_eq!(item.sha1.as_deref(), Some("cafef00d"));
        assert_eq!(item.size, Some(1000));
    }

    #[test]
    fn client_jar_item_none_without_download() {
        let profile = profile_from(r#"{"id":"x","libraries":[]}"#);
        let paths = GamePaths::new("/games/mc");
        assert!(client_jar_item(&profile, &paths).is_none());
    }
}
