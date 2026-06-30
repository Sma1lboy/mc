//! Modrinth `.mrpack` 索引模型(`modrinth.index.json`)。
//!
//! 布局:zip,根有唯一 `modrinth.index.json`(`formatVersion==1`、`game=="minecraft"`)
//! + 可选 `overrides/`(客户端 + 服务端都铺)、`client-overrides/`(仅客户端,盖在
//!   overrides 上)。`files[]` 里的文件**不在包内**,要按 `downloads[]` 下载。
//!
//! 字段名严格对齐 Modrinth modpack 规范(<https://docs.modrinth.com/modpacks/format/>)。
//! 易错点(对照 `docs/modules/modpack-formats.md` §1):
//! - **Prism 强制 `sha512`**(校验 + 更新去重键);`sha1` 可缺。
//! - `env.client == Unsupported` → 整个文件跳过;`== Optional` → 降级为可选。导入只看 client。
//! - 下载 host 白名单:`cdn.modrinth.com / github.com / raw.githubusercontent.com / gitlab.com`。
//! - `MrpackDependencies` 用 `deny_unknown_fields` 复刻 Prism「Unknown dependency type」抛错。

use serde::{Deserialize, Serialize};

/// Modrinth modpack 规范允许的下载 host 白名单(<https://docs.modrinth.com/modpacks/format/>)。
///
/// **单一真相源**:导入侧用它过滤 `files[].downloads[]` 的源(纵深防御,丢弃指向任意
/// host 的恶意源),导出侧用它作远程引用门(host 不在其中则回落 `overrides/`)。匹配规则
/// 是「等于某项或为其子域」(见 [`crate::host::host_matches_suffix`]),不是裸后缀,避免
/// `evilmodrinth.com` 蒙混。
pub const MRPACK_DOWNLOAD_HOSTS: &[&str] = &[
    "cdn.modrinth.com",
    "github.com",
    "raw.githubusercontent.com",
    "gitlab.com",
];

/// `modrinth.index.json` 顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackIndex {
    /// 格式版本,当前规范为 1。
    #[serde(rename = "formatVersion")]
    pub format_version: u32,
    /// 必须为 `"minecraft"`。
    pub game: String,
    /// 整合包版本号(自由文本,如 `1.0.0`)。
    #[serde(rename = "versionId", default)]
    pub version_id: String,
    /// 整合包名。
    pub name: String,
    /// 可选简介。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 依赖:必含 `minecraft`,可含一种 loader。
    pub dependencies: MrpackDependencies,
    /// 受管理的远程文件列表(不在包内,要下载)。
    #[serde(default)]
    pub files: Vec<MrpackFile>,
}

/// `dependencies` 子对象。`deny_unknown_fields` 复刻 Prism 对未知依赖类型抛错的行为
/// (避免静默忽略一个我们无法安装的 loader)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MrpackDependencies {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft: Option<String>,
    #[serde(rename = "fabric-loader", default, skip_serializing_if = "Option::is_none")]
    pub fabric_loader: Option<String>,
    #[serde(rename = "quilt-loader", default, skip_serializing_if = "Option::is_none")]
    pub quilt_loader: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neoforge: Option<String>,
}

/// `files[]` 中的单个受管理文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MrpackFile {
    /// 相对游戏目录的落盘路径,如 `mods/sodium.jar`;反斜杠应归一为 `/`;必过 `safe_join`。
    pub path: String,
    /// 校验和(`sha512` 必有,`sha1` 可缺)。
    pub hashes: MrpackHashes,
    /// 环境适用性。缺省视为客户端 + 服务端都需要。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<MrpackEnv>,
    /// 有序候选 URL,非空;hosts 受白名单约束(纵深防御)。
    pub downloads: Vec<String>,
    /// 文件大小(字节),用于进度 / 校验声明大小。
    #[serde(rename = "fileSize", default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
}

/// 文件校验和。`sha512` 是规范哈希(Prism 强制),`sha1` 可缺。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MrpackHashes {
    /// Modrinth 整合包的规范哈希;下载后优先用它强校验。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sha512: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha1: Option<String>,
}

/// `files[].env`:客户端 / 服务端各自的适用性。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MrpackEnv {
    pub client: EnvSupport,
    pub server: EnvSupport,
}

/// 单端的适用性。`Unsupported` → 跳过;`Optional` → 降级为可选;`Required` → 必装。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvSupport {
    Required,
    Optional,
    Unsupported,
}

impl MrpackFile {
    /// 该文件在客户端是否受支持(用于跳过纯服务端文件)。
    ///
    /// 仅当显式标注 `env.client == Unsupported` 时跳过;缺省 / 其它取值都视为需要。
    pub fn client_supported(&self) -> bool {
        match &self.env {
            Some(env) => env.client != EnvSupport::Unsupported,
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_index_with_renames_and_env() {
        let sample = r#"{
            "formatVersion": 1,
            "game": "minecraft",
            "name": "My Modpack",
            "versionId": "1.0.0",
            "summary": "a pack",
            "dependencies": {
                "minecraft": "1.20.1",
                "fabric-loader": "0.15.7"
            },
            "files": [
                {
                    "path": "mods/sodium.jar",
                    "downloads": ["https://cdn.modrinth.com/data/x/sodium.jar"],
                    "hashes": { "sha512": "longhash", "sha1": "deadbeef" },
                    "fileSize": 123456,
                    "env": { "client": "required", "server": "optional" }
                },
                {
                    "path": "mods/server-only.jar",
                    "downloads": ["https://example.com/server-only.jar"],
                    "hashes": { "sha512": "h2" },
                    "env": { "client": "unsupported", "server": "required" }
                }
            ]
        }"#;

        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.format_version, 1);
        assert_eq!(index.game, "minecraft");
        assert_eq!(index.version_id, "1.0.0");
        assert_eq!(index.name, "My Modpack");
        assert_eq!(index.summary.as_deref(), Some("a pack"));
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(index.dependencies.fabric_loader.as_deref(), Some("0.15.7"));
        assert_eq!(index.files.len(), 2);

        let f0 = &index.files[0];
        assert_eq!(f0.path, "mods/sodium.jar");
        assert_eq!(f0.hashes.sha512, "longhash");
        assert_eq!(f0.hashes.sha1.as_deref(), Some("deadbeef"));
        assert_eq!(f0.file_size, Some(123456));
        assert!(f0.client_supported());

        // client == unsupported → 跳过。
        assert!(!index.files[1].client_supported());
        assert!(index.files[1].hashes.sha1.is_none());
    }

    #[test]
    fn minimal_index_no_files_no_env() {
        let sample = r#"{
            "formatVersion": 1,
            "game": "minecraft",
            "name": "Mini",
            "dependencies": { "minecraft": "1.21" }
        }"#;
        let index: MrpackIndex = serde_json::from_str(sample).unwrap();
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.21"));
        assert!(index.files.is_empty());
        assert!(index.summary.is_none());
    }

    #[test]
    fn dependencies_reject_unknown_loader_key() {
        // deny_unknown_fields:未知 loader 键(如未来 loader)直接报错,而非静默忽略。
        let sample = r#"{ "minecraft": "1.20.1", "unknown-loader": "1.0" }"#;
        let parsed: std::result::Result<MrpackDependencies, _> = serde_json::from_str(sample);
        assert!(parsed.is_err(), "未知依赖键应被 deny_unknown_fields 拒绝");
    }

    #[test]
    fn roundtrip_preserves_required_fields() {
        let index = MrpackIndex {
            format_version: 1,
            game: "minecraft".to_string(),
            version_id: "2.0".to_string(),
            name: "RT".to_string(),
            summary: None,
            dependencies: MrpackDependencies {
                minecraft: Some("1.20.1".to_string()),
                neoforge: Some("47.1.0".to_string()),
                ..Default::default()
            },
            files: vec![MrpackFile {
                path: "mods/a.jar".to_string(),
                hashes: MrpackHashes { sha512: "h".to_string(), sha1: None },
                env: Some(MrpackEnv { client: EnvSupport::Required, server: EnvSupport::Required }),
                downloads: vec!["https://cdn.modrinth.com/a.jar".to_string()],
                file_size: Some(10),
            }],
        };
        let json = serde_json::to_string(&index).unwrap();
        // 验证 camelCase 键名被正确发出。
        assert!(json.contains("\"formatVersion\":1"));
        assert!(json.contains("\"versionId\":\"2.0\""));
        assert!(json.contains("\"fileSize\":10"));
        // summary == None 时不应出现键。
        assert!(!json.contains("summary"));

        let back: MrpackIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(back.dependencies.neoforge.as_deref(), Some("47.1.0"));
        assert_eq!(back.files[0].downloads.len(), 1);
    }
}
