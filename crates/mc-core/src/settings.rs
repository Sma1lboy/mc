//! 全局启动器设置。
//!
//! 这里保存的是**跨实例**的用户偏好(下载源、并发数、默认内存、界面语言等),
//! 持久化为 `data_dir/settings.json`。它与 [`crate::instance::InstanceConfig`]
//! 不同:后者是单个游戏实例的配置,前者是整个启动器的全局默认值。
//!
//! 设计要点:
//! - **永不因配置坏掉而启动失败**:文件缺失或 JSON 损坏时都回退到 [`Default`],
//!   只记一条 warn 日志,绝不向上抛错。用户的损坏配置不应阻止启动器运行。
//! - 写入走 [`crate::fs::write_atomic`],崩溃中途不会留下半截文件。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::download::MirrorResolver;
use crate::error::Result;

/// 设置文件名(位于 `data_dir` 下)。
const SETTINGS_FILE: &str = "settings.json";

/// 默认下载源标识(直连 Mojang 官方 CDN)。
const SOURCE_OFFICIAL: &str = "official";
/// BMCLAPI 镜像源标识。
const SOURCE_BMCLAPI: &str = "bmclapi";

/// 默认下载并发数。BMCLAPI 与官方源都能轻松扛住,数百小文件下并发越高越快。
const DEFAULT_CONCURRENCY: usize = 64;
/// 默认分配给游戏的堆内存(MiB)。2 GiB 适合原版 + 轻量 mod。
const DEFAULT_MEMORY_MB: u32 = 2048;
/// 默认界面语言。
const DEFAULT_LANGUAGE: &str = "zh-CN";

/// 跨实例的全局启动器设置。
///
/// 所有字段都有合理默认值(见 [`Default`] 实现),所以即便 `settings.json`
/// 只写了部分字段,缺失的字段也会通过 serde 默认值补齐(配合 `#[serde(default)]`)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSettings {
    /// 下载源:`"official"`(官方)或 `"bmclapi"`(镜像)。
    #[serde(default = "default_download_source")]
    pub download_source: String,

    /// 下载并发数。
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,

    /// 新建实例的默认堆内存(MiB)。
    #[serde(default = "default_memory_mb")]
    pub default_memory_mb: u32,

    /// 全局 Java 可执行文件路径。`None` 表示让启动器自动探测/安装。
    #[serde(default)]
    pub java_path: Option<String>,

    /// 是否启用下载镜像。即使 `download_source` 是官方,该开关也能强制走镜像;
    /// 二者任一指向镜像即生效(见 [`GlobalSettings::mirror_resolver`])。
    #[serde(default)]
    pub use_mirror: bool,

    /// 界面语言标签(如 `"zh-CN"`、`"en-US"`)。
    #[serde(default = "default_language")]
    pub language: String,

    /// 可选的远端服务地址(例如皮肤站/账号服务/自建同步服务)。
    #[serde(default)]
    pub server_url: Option<String>,

    /// 额外的自定义数据根目录列表(多游戏根/外部实例库)。
    #[serde(default)]
    pub custom_roots: Vec<String>,
}

fn default_download_source() -> String {
    SOURCE_OFFICIAL.to_string()
}
fn default_concurrency() -> usize {
    DEFAULT_CONCURRENCY
}
fn default_memory_mb() -> u32 {
    DEFAULT_MEMORY_MB
}
fn default_language() -> String {
    DEFAULT_LANGUAGE.to_string()
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            download_source: default_download_source(),
            concurrency: default_concurrency(),
            default_memory_mb: default_memory_mb(),
            java_path: None,
            use_mirror: false,
            language: default_language(),
            server_url: None,
            custom_roots: Vec::new(),
        }
    }
}

impl GlobalSettings {
    /// `data_dir` 下设置文件的完整路径。
    pub fn path(data_dir: &Path) -> PathBuf {
        data_dir.join(SETTINGS_FILE)
    }

    /// 从 `data_dir/settings.json` 读取设置。
    ///
    /// **永不报错**:
    /// - 文件不存在 → 返回 [`Default`]。
    /// - 文件存在但读取失败(权限等)→ 记 warn,返回 [`Default`]。
    /// - 文件存在但 JSON 损坏 → 记 warn,返回 [`Default`]。
    ///
    /// 返回类型保留 [`Result`] 以保持 API 一致性,但当前实现总是 `Ok`。
    pub fn load(data_dir: &Path) -> Result<Self> {
        let path = Self::path(data_dir);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // 首次运行:没有配置文件是正常的,静默回退默认值。
                return Ok(Self::default());
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e,
                    "读取 settings.json 失败,回退到默认设置");
                return Ok(Self::default());
            }
        };

        match serde_json::from_slice::<Self>(&bytes) {
            Ok(s) => Ok(s),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e,
                    "解析 settings.json 失败,回退到默认设置");
                Ok(Self::default())
            }
        }
    }

    /// 把设置原子写入 `data_dir/settings.json`,自动创建缺失的目录。
    ///
    /// 序列化为带缩进的 JSON(便于人工查看/编辑),写入走
    /// [`crate::fs::write_atomic`](crate::fs::write_atomic),崩溃安全。
    pub fn save(&self, data_dir: &Path) -> Result<()> {
        let path = Self::path(data_dir);
        let json = serde_json::to_vec_pretty(self).map_err(|e| crate::error::CoreError::Parse {
            what: "settings.json".to_string(),
            source: e,
        })?;
        // write_atomic 内部已 create_dir_all 父目录,无需额外建目录。
        crate::fs::write_atomic(&path, &json)
    }

    /// 根据 `download_source` / `use_mirror` 推导镜像改写器。
    ///
    /// 只要二者任一指向镜像(`download_source == "bmclapi"` 或 `use_mirror == true`),
    /// 就返回 BMCLAPI 改写器;否则返回直连官方源的空改写器。
    pub fn mirror_resolver(&self) -> MirrorResolver {
        let wants_mirror = self.use_mirror || self.download_source.eq_ignore_ascii_case(SOURCE_BMCLAPI);
        if wants_mirror {
            MirrorResolver::bmclapi()
        } else {
            MirrorResolver::none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在临时目录里造一个唯一子目录,测完调用方负责清理。
    fn temp_data_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        // 用进程 id + 纳秒时间戳避免并发测试相互踩踏。
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(format!("mc-core-settings-test-{}-{}", std::process::id(), nanos));
        dir
    }

    #[test]
    fn default_values_are_sane() {
        let s = GlobalSettings::default();
        assert_eq!(s.download_source, "official");
        assert_eq!(s.concurrency, 64);
        assert_eq!(s.default_memory_mb, 2048);
        assert!(s.java_path.is_none());
        assert!(!s.use_mirror);
        assert_eq!(s.language, "zh-CN");
        assert!(s.server_url.is_none());
        assert!(s.custom_roots.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_data_dir();
        let mut s = GlobalSettings::default();
        s.download_source = "bmclapi".to_string();
        s.concurrency = 16;
        s.default_memory_mb = 4096;
        s.java_path = Some("/opt/java/bin/java".to_string());
        s.use_mirror = true;
        s.language = "en-US".to_string();
        s.server_url = Some("https://example.com".to_string());
        s.custom_roots = vec!["/games/a".to_string(), "/games/b".to_string()];

        // 目录尚不存在,save 必须自动创建。
        assert!(!dir.exists());
        s.save(&dir).expect("save should succeed");
        assert!(GlobalSettings::path(&dir).exists());

        let loaded = GlobalSettings::load(&dir).expect("load should succeed");
        assert_eq!(loaded.download_source, s.download_source);
        assert_eq!(loaded.concurrency, s.concurrency);
        assert_eq!(loaded.default_memory_mb, s.default_memory_mb);
        assert_eq!(loaded.java_path, s.java_path);
        assert_eq!(loaded.use_mirror, s.use_mirror);
        assert_eq!(loaded.language, s.language);
        assert_eq!(loaded.server_url, s.server_url);
        assert_eq!(loaded.custom_roots, s.custom_roots);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = temp_data_dir();
        // 目录/文件都不存在。
        assert!(!GlobalSettings::path(&dir).exists());
        let loaded = GlobalSettings::load(&dir).expect("load should not error");
        // 与默认值逐字段一致。
        let def = GlobalSettings::default();
        assert_eq!(loaded.download_source, def.download_source);
        assert_eq!(loaded.concurrency, def.concurrency);
        assert_eq!(loaded.default_memory_mb, def.default_memory_mb);
        assert_eq!(loaded.language, def.language);
        // 不应有副作用:load 不创建文件。
        assert!(!GlobalSettings::path(&dir).exists());
    }

    #[test]
    fn corrupt_json_returns_default() {
        let dir = temp_data_dir();
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = GlobalSettings::path(&dir);
        std::fs::write(&path, b"{ this is not valid json ]]]").expect("write garbage");

        let loaded = GlobalSettings::load(&dir).expect("load must not error on garbage");
        // 回退到默认值。
        assert_eq!(loaded.download_source, "official");
        assert_eq!(loaded.concurrency, 64);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_json_fills_defaults() {
        let dir = temp_data_dir();
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = GlobalSettings::path(&dir);
        // 只写一个字段,其余应由 serde 默认值补齐。
        std::fs::write(&path, br#"{"concurrency": 8}"#).expect("write partial");

        let loaded = GlobalSettings::load(&dir).expect("load partial");
        assert_eq!(loaded.concurrency, 8);
        assert_eq!(loaded.download_source, "official"); // 默认补齐
        assert_eq!(loaded.default_memory_mb, 2048); // 默认补齐
        assert_eq!(loaded.language, "zh-CN"); // 默认补齐

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mirror_resolver_picks_bmclapi_when_source_is_bmclapi() {
        let mut s = GlobalSettings::default();
        s.download_source = "bmclapi".to_string();
        s.use_mirror = false;
        // 官方 URL 应被改写(命中镜像规则 → 字符串改变)。
        let original = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
        let rewritten = s.mirror_resolver().rewrite(original);
        assert_ne!(rewritten, original, "bmclapi 源应改写官方 URL");
    }

    #[test]
    fn mirror_resolver_picks_bmclapi_when_use_mirror_flag_set() {
        let mut s = GlobalSettings::default();
        s.download_source = "official".to_string();
        s.use_mirror = true;
        let original = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
        let rewritten = s.mirror_resolver().rewrite(original);
        assert_ne!(rewritten, original, "use_mirror 开启时应改写官方 URL");
    }

    #[test]
    fn mirror_resolver_is_none_for_official() {
        let s = GlobalSettings::default(); // official + use_mirror=false
        let original = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
        let rewritten = s.mirror_resolver().rewrite(original);
        assert_eq!(rewritten, original, "官方源应直连,不改写 URL");
    }
}
