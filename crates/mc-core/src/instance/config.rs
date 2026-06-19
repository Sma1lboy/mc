//! 每实例的可覆盖设置 (`<version_dir>/instance.json`)。
//!
//! 采用"版本即实例"模型:每个 `versions/<id>` 目录就是一个实例,实例的
//! 启动设置(内存、Java、窗口、附加参数)存放在该目录下的 `instance.json`。
//! 这是对全局默认的覆盖层 —— 字段大多用 `Option`/默认值,缺省即回退到全局。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::paths::ensure_dir;

/// 单个实例的启动设置。所有字段在 json 缺失时回退到 [`Default`],
/// 因此向 `instance.json` 增量添加字段不会破坏旧文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstanceConfig {
    /// 实例展示名;缺省时由上层用版本 id 兜底。
    pub name: Option<String>,
    /// 分配给 JVM 的最大堆内存 (MB)。默认 2048。
    pub memory_mb: u32,
    /// 固定使用的 Java 可执行文件路径;缺省走全局 Java 选择逻辑。
    pub java_path: Option<String>,
    /// 追加到自动生成 JVM 参数之后的额外参数。
    pub jvm_args: Vec<String>,
    /// 追加到自动生成游戏参数之后的额外参数。
    pub game_args: Vec<String>,
    /// 游戏窗口宽度 (像素);缺省用全局/游戏默认。
    pub width: Option<u32>,
    /// 游戏窗口高度 (像素);缺省用全局/游戏默认。
    pub height: Option<u32>,
    /// 是否以全屏启动。
    pub fullscreen: bool,
    /// 启动即自动加入的服务器地址 (`host` 或 `host:port`)。
    pub server: Option<String>,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            name: None,
            // 2048 MB 是一个对原版/轻度模组都安全的默认堆上限。
            memory_mb: 2048,
            java_path: None,
            jvm_args: Vec::new(),
            game_args: Vec::new(),
            width: None,
            height: None,
            fullscreen: false,
            server: None,
        }
    }
}

impl InstanceConfig {
    /// 从 `path` 读取实例配置。
    ///
    /// 文件不存在视为"尚未自定义",返回 [`InstanceConfig::default`] 而非报错 ——
    /// 这样新建实例无需先写盘即可启动。仅当文件存在但读取/解析失败时才返回错误。
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| crate::error::CoreError::Parse { what: "instance.json".to_string(), source: e }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(crate::error::CoreError::io(path, e)),
        }
    }

    /// 将配置序列化为美化 json 写入 `path`,自动创建父目录。
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            ensure_dir(parent)?;
        }
        // 用美化输出,方便用户手动编辑 instance.json。
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| crate::error::CoreError::Parse { what: "instance.json".to_string(), source: e })?;
        crate::fs::write_atomic(path, json.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_has_sane_memory() {
        let c = InstanceConfig::default();
        assert_eq!(c.memory_mb, 2048);
        assert!(!c.fullscreen);
        assert!(c.name.is_none());
        assert!(c.jvm_args.is_empty());
    }

    #[test]
    fn load_missing_returns_default() {
        let path = std::env::temp_dir().join("mc-core-instance-cfg-missing-xyz.json");
        // 确保不存在。
        let _ = std::fs::remove_file(&path);
        let c = InstanceConfig::load(&path).unwrap();
        assert_eq!(c, InstanceConfig::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = std::env::temp_dir().join("mc-core-instance-cfg-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("instance.json");

        let mut cfg = InstanceConfig::default();
        cfg.name = Some("My Pack".to_string());
        cfg.memory_mb = 4096;
        cfg.jvm_args = vec!["-XX:+UseG1GC".to_string()];
        cfg.width = Some(1280);
        cfg.height = Some(720);
        cfg.fullscreen = true;
        cfg.server = Some("mc.example.com:25565".to_string());

        cfg.save(&path).unwrap();
        let loaded = InstanceConfig::load(&path).unwrap();
        assert_eq!(loaded, cfg);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_json_fills_defaults() {
        // 只给出 memory_mb,其余字段应回退默认。
        let cfg: InstanceConfig = serde_json::from_str(r#"{"memory_mb":3072}"#).unwrap();
        assert_eq!(cfg.memory_mb, 3072);
        assert!(cfg.name.is_none());
        assert!(!cfg.fullscreen);
        assert!(cfg.game_args.is_empty());
    }
}
