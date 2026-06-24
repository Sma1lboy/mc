//! 读取实例的多人服务器列表(`servers.dat`)。
//!
//! `servers.dat` 是**未压缩**的 NBT(与 gzip 压缩的 `level.dat` 不同):根复合标签下有一个
//! `servers` 列表,每项含 `name`(显示名)/ `ip`(地址,可带 `:端口`)/ `icon`(64×64 PNG 的
//! base64,可缺)。解析尽量宽容:单项缺 `ip` 跳过,文件不存在返回空表(不是错误)。

use std::collections::HashMap;
use std::path::Path;

use fastnbt::Value;
use serde::Serialize;

use crate::error::{CoreError, IoResultExt, Result};

/// `servers.dat` 文件名。
const SERVERS_DAT: &str = "servers.dat";

/// 一条已保存的多人服务器记录(展示 + 快速进入用)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct SavedServer {
    /// 显示名(可空 —— UI 用地址兜底)。
    pub name: String,
    /// 服务器地址(`host` 或 `host:port`)。
    pub address: String,
    /// 服务器图标:64×64 PNG 的 base64(不含 `data:` 前缀);无则 `None`。
    pub icon: Option<String>,
}

/// 读取 `game_dir/servers.dat` 的服务器列表。文件不存在 → 空表;解析失败 → 错误。
pub fn read_servers(game_dir: &Path) -> Result<Vec<SavedServer>> {
    let path = game_dir.join(SERVERS_DAT);
    let buf = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(CoreError::io(&path, e)),
    };
    // 规范为未压缩 NBT;个别外部工具可能 gzip,故先按原始解,失败再回退 gzip 解一次。
    let root: Value = match fastnbt::from_bytes::<Value>(&buf) {
        Ok(v) => v,
        Err(_) => {
            use flate2::read::GzDecoder;
            use std::io::Read;
            let mut out = Vec::new();
            GzDecoder::new(&buf[..])
                .read_to_end(&mut out)
                .with_path(&path)?;
            fastnbt::from_bytes::<Value>(&out)
                .map_err(|e| CoreError::other(format!("解析 servers.dat NBT 失败: {e}")))?
        }
    };
    Ok(extract_servers(&root))
}

/// 从已解析的 root NBT 抽出服务器列表(纯函数,可单测)。
fn extract_servers(root: &Value) -> Vec<SavedServer> {
    let Value::Compound(map) = root else {
        return Vec::new();
    };
    let Some(Value::List(list)) = map.get("servers") else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let Value::Compound(s) = entry else {
                return None;
            };
            let address = str_field(s, "ip")?;
            if address.trim().is_empty() {
                return None;
            }
            Some(SavedServer {
                name: str_field(s, "name").unwrap_or_default(),
                address,
                icon: str_field(s, "icon").filter(|x| !x.is_empty()),
            })
        })
        .collect()
}

/// 取一个 NBT 复合标签里的字符串字段(非字符串 / 缺失 → `None`)。
fn str_field(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compound(pairs: Vec<(&str, Value)>) -> Value {
        Value::Compound(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn extracts_name_ip_icon_and_skips_entries_without_ip() {
        let root = compound(vec![(
            "servers",
            Value::List(vec![
                compound(vec![
                    ("name", Value::String("Hypixel".into())),
                    ("ip", Value::String("mc.hypixel.net".into())),
                    ("icon", Value::String("BASE64ICON".into())),
                ]),
                // 无 ip → 跳过。
                compound(vec![("name", Value::String("No IP".into()))]),
                // 无 name / icon → 名留空、icon None,但保留(有 ip)。
                compound(vec![("ip", Value::String("play.example.com:25566".into()))]),
            ]),
        )]);
        let servers = extract_servers(&root);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].name, "Hypixel");
        assert_eq!(servers[0].address, "mc.hypixel.net");
        assert_eq!(servers[0].icon.as_deref(), Some("BASE64ICON"));
        assert_eq!(servers[1].name, "");
        assert_eq!(servers[1].address, "play.example.com:25566");
        assert_eq!(servers[1].icon, None);
    }

    #[test]
    fn missing_servers_key_is_empty() {
        assert!(extract_servers(&compound(vec![])).is_empty());
        // 空 ip 串也跳过。
        let root = compound(vec![(
            "servers",
            Value::List(vec![compound(vec![("ip", Value::String("".into()))])]),
        )]);
        assert!(extract_servers(&root).is_empty());
    }
}
