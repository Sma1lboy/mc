//! MCBBS(国内)/ HMCL-lineage 整合包格式。
//!
//! zip,根 `mcbbs.packmeta`(或带 `addons` 的 `manifest.json`)+ `overrides/`。mod 不在
//! 包内,经 CurseForge / Modrinth 拉。
//!
//! 易错点(对照 `docs/modules/modpack-formats.md` §5):
//! - **靠 `addons` 存在区分于 CurseForge**(同名 `manifest.json` 时读内容判别)。
//! - `addons[id=="game"]` 是 MC 版本(必需);其余 id 是 loader(forge/neoforge/fabric/quilt/optifine/liteloader)。
//! - `files[]` 是 CurseForge-shaped `{projectID,fileID,type,...}`。
//! - `launchInfo.{launch_argument,java_argument}` → 实例游戏 / JVM 参数。
//! - 野包常缺 `manifestType` / `manifestVersion`,别强求。

use serde::{Deserialize, Serialize};

/// `mcbbs.packmeta`(或带 `addons` 的 `manifest.json`)顶层结构。
///
/// 几乎所有字段都 `Option` / `default`:野包字段缺失很常见,不让单字段缺失打挂解析。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McbbsPackMeta {
    /// `"minecraftModpack"`(野包常缺,别强求)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// 更新 / 镜像基址。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_override: Option<bool>,
    /// CurseForge-shaped 文件列表。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<McbbsFile>,
    /// 加载器谱:扁平 `id → version`。**区分 CF 的判别字段**。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addons: Vec<McbbsAddon>,
    /// 启动参数 / 内存 / Java 等。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_info: Option<McbbsLaunchInfo>,
    /// 杂项设置(首装可忽略;原样保留以便无损)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
}

/// `files[]` 中的一项:CurseForge-shaped `{projectID,fileID,type,...}`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McbbsFile {
    #[serde(rename = "projectID", default)]
    pub project_id: i64,
    #[serde(rename = "fileID", default)]
    pub file_id: i64,
    /// 类型(如 `"curse"`)。
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 是否必需(默认 true,语义同 CF;野包偶尔显式给)。
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

/// `addons[]` 中的一项:扁平 `id → version`。
///
/// `id` ∈ `"game"`(必需,MC 版本) | `forge` | `neoforge` | `fabric` | `quilt` | `optifine` | `liteloader`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McbbsAddon {
    pub id: String,
    pub version: String,
}

/// `launchInfo` 子对象:启动参数 / 内存 / Java。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McbbsLaunchInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_memory: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_java_versions: Vec<u32>,
    /// → 实例游戏参数。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub launch_argument: Vec<String>,
    /// → 实例 JVM 参数。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub java_argument: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_launch_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_exit_command: Option<String>,
}

impl McbbsPackMeta {
    /// MC 版本 = `addons[id=="game"].version`(必需)。
    pub fn minecraft_version(&self) -> Option<&str> {
        self.addons
            .iter()
            .find(|a| a.id == "game")
            .map(|a| a.version.as_str())
    }

    /// 取除 `game` 外的 loader addon(forge / neoforge / fabric / quilt / optifine / liteloader)。
    pub fn loader_addons(&self) -> impl Iterator<Item = &McbbsAddon> {
        self.addons.iter().filter(|a| a.id != "game")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_packmeta_with_addons_and_launch_info() {
        let sample = r#"{
            "manifestType": "minecraftModpack",
            "manifestVersion": 1,
            "name": "国服整合",
            "author": "someone",
            "fileApi": "https://mirror.example/files",
            "addons": [
                { "id": "game", "version": "1.20.1" },
                { "id": "forge", "version": "47.2.0" }
            ],
            "files": [
                { "projectID": 238222, "fileID": 4567890, "type": "curse" }
            ],
            "launchInfo": {
                "minMemory": 4096,
                "supportedJavaVersions": [17, 21],
                "launchArgument": ["--fullscreen"],
                "javaArgument": ["-XX:+UseG1GC"]
            }
        }"#;
        let m: McbbsPackMeta = serde_json::from_str(sample).unwrap();
        assert_eq!(m.name.as_deref(), Some("国服整合"));
        assert_eq!(m.file_api.as_deref(), Some("https://mirror.example/files"));
        assert_eq!(m.minecraft_version(), Some("1.20.1"));

        let loaders: Vec<_> = m.loader_addons().collect();
        assert_eq!(loaders.len(), 1);
        assert_eq!(loaders[0].id, "forge");
        assert_eq!(loaders[0].version, "47.2.0");

        assert_eq!(m.files[0].project_id, 238222);
        assert_eq!(m.files[0].kind.as_deref(), Some("curse"));
        assert!(m.files[0].required, "type-shaped file 默认 required true");

        let li = m.launch_info.as_ref().unwrap();
        assert_eq!(li.min_memory, Some(4096));
        assert_eq!(li.supported_java_versions, vec![17, 21]);
        assert_eq!(li.launch_argument, vec!["--fullscreen".to_string()]);
        assert_eq!(li.java_argument, vec!["-XX:+UseG1GC".to_string()]);
    }

    #[test]
    fn wild_packmeta_missing_manifest_fields_still_parses() {
        // 野包常缺 manifestType/Version;addons 仍能给出 MC 版本。
        let sample = r#"{ "name": "wild", "addons": [ { "id": "game", "version": "1.19.2" } ] }"#;
        let m: McbbsPackMeta = serde_json::from_str(sample).unwrap();
        assert!(m.manifest_type.is_none());
        assert_eq!(m.minecraft_version(), Some("1.19.2"));
        assert_eq!(m.loader_addons().count(), 0);
    }
}
