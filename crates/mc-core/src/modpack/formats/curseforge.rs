//! CurseForge 整合包 `manifest.json` 模型。
//!
//! zip 根:`manifest.json`(`manifestType=="minecraftModpack"`、`manifestVersion==1`)
//! + `overrides/`(名由 `manifest.overrides` 定,默认 `overrides`)+ 可选 `modlist.html`
//!   (忽略)。`files[]` **只给 projectID/fileID**,要经 Flame API 解析为真实 URL ——
//!   解析时复用 [`crate::modplatform::curseforge::FlameApiFile`](本模块**不**重复定义那组
//!   API 结构,只建模 `manifest.json` 自身)。
//!
//! 易错点(对照 `docs/modules/modpack-formats.md` §3):
//! - `manifestType`/`manifestVersion` 不符直接拒,不做尽力解析(会错配 id)。
//! - MC 版本在 `minecraft.version`,**不在** loader id 里。
//! - `modLoaders[].id` 形如 `forge-47.2.0` / `fabric-0.15.7`,`split_once('-')` 分家族与版本。
//! - `recommendedRam` 是 camelCase。
//! - `files[].required` 默认 **true**,是 optional 的反义,极易弄反。
//! - 区分 MCBBS:CF manifest **无** `addons`/`launchInfo`;有则按 MCBBS 解析(见 [`super::mcbbs`])。

use serde::{Deserialize, Serialize};

fn default_pack_name() -> String {
    "Unnamed".to_string()
}
fn default_author() -> String {
    "Anonymous".to_string()
}
fn default_overrides() -> String {
    "overrides".to_string()
}
fn default_true() -> bool {
    true
}

/// `manifest.json` 顶层结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameManifest {
    /// 必须为 `"minecraftModpack"`。
    pub manifest_type: String,
    /// 必须为 1。
    pub manifest_version: i32,
    /// MC 版本 + loader 谱。
    pub minecraft: FlameMinecraft,
    /// 整合包名(默认 `"Unnamed"`)。
    #[serde(default = "default_pack_name")]
    pub name: String,
    /// 版本号(自由文本)。
    #[serde(default)]
    pub version: String,
    /// 作者(默认 `"Anonymous"`)。
    #[serde(default = "default_author")]
    pub author: String,
    /// 受管理文件列表(只给 projectID/fileID)。
    #[serde(default)]
    pub files: Vec<FlameManifestFile>,
    /// overrides 目录名(默认 `"overrides"`)。
    #[serde(default = "default_overrides")]
    pub overrides: String,
}

/// `minecraft` 子对象:MC 版本 + loader 谱。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlameMinecraft {
    /// MC 版本(loader id 里**没有**它)。
    pub version: String,
    /// 1.2.5 FTB 遗留字段,忽略。
    #[serde(default)]
    pub libraries: String,
    /// loader 谱(通常一项,`primary==true` 的那个生效)。
    #[serde(default)]
    pub mod_loaders: Vec<FlameModLoader>,
    /// 推荐内存(MB)。JSON 键是 camelCase `recommendedRam`。
    #[serde(default)]
    pub recommended_ram: i32,
}

/// 一条 loader 声明。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlameModLoader {
    /// `"forge-47.2.0"` / `"fabric-0.15.7"` / `"neoforge-..."` / `"quilt-..."`,`split_once('-')`。
    pub id: String,
    /// 默认 false;装 primary 的那个(无则取第一个)。
    #[serde(default)]
    pub primary: bool,
}

impl FlameModLoader {
    /// 把 `id` 切成 (loader 家族小写, 版本)。无 `-` 时整体当家族、版本为空。
    ///
    /// 例:`"forge-47.2.0"` → `("forge", "47.2.0")`;`"neoforge-21.0.0"` → `("neoforge", "21.0.0")`。
    pub fn split_id(&self) -> (String, String) {
        match self.id.split_once('-') {
            Some((fam, ver)) => (fam.to_ascii_lowercase(), ver.to_string()),
            None => (self.id.to_ascii_lowercase(), String::new()),
        }
    }
}

impl FlameMinecraft {
    /// 选出生效的 loader:优先 `primary==true`,否则第一个。
    pub fn primary_loader(&self) -> Option<&FlameModLoader> {
        self.mod_loaders
            .iter()
            .find(|l| l.primary)
            .or_else(|| self.mod_loaders.first())
    }
}

/// `files[]` 中的一项:只给 projectID/fileID,要经 Flame API 解析。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FlameManifestFile {
    /// CurseForge 项目 id。
    #[serde(rename = "projectID")]
    pub project_id: i64,
    /// CurseForge 文件 id。
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// 默认 **TRUE**,是 `optional` 的反义,极易弄反。
    #[serde(default = "default_true")]
    pub required: bool,
}

impl FlameManifest {
    /// 该 manifest 的 `manifestType` / `manifestVersion` 是否符合 CF 整合包规范。
    /// 不符直接拒(见模块文档:不做尽力解析)。
    pub fn is_valid(&self) -> bool {
        self.manifest_type == "minecraftModpack" && self.manifest_version == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest_with_defaults_and_required_true() {
        let sample = r#"{
            "manifestType": "minecraftModpack",
            "manifestVersion": 1,
            "minecraft": {
                "version": "1.20.1",
                "modLoaders": [
                    { "id": "forge-47.2.0", "primary": true }
                ],
                "recommendedRam": 6144
            },
            "files": [
                { "projectID": 238222, "fileID": 4567890 },
                { "projectID": 12345, "fileID": 999, "required": false }
            ]
        }"#;

        let m: FlameManifest = serde_json::from_str(sample).unwrap();
        assert!(m.is_valid());
        // 默认值。
        assert_eq!(m.name, "Unnamed");
        assert_eq!(m.author, "Anonymous");
        assert_eq!(m.overrides, "overrides");
        // MC 版本在 minecraft.version。
        assert_eq!(m.minecraft.version, "1.20.1");
        assert_eq!(m.minecraft.recommended_ram, 6144);

        let loader = m.minecraft.primary_loader().unwrap();
        assert_eq!(loader.split_id(), ("forge".to_string(), "47.2.0".to_string()));

        // required 默认 true(第 0 个未给),第 1 个显式 false。
        assert!(m.files[0].required, "未给 required 应默认 true");
        assert!(!m.files[1].required);
        assert_eq!(m.files[0].project_id, 238222);
        assert_eq!(m.files[0].file_id, 4567890);
    }

    #[test]
    fn custom_overrides_dir_and_neoforge_split() {
        let sample = r#"{
            "manifestType": "minecraftModpack",
            "manifestVersion": 1,
            "name": "Custom",
            "overrides": "src",
            "minecraft": {
                "version": "1.20.1",
                "modLoaders": [ { "id": "neoforge-47.1.0" } ]
            }
        }"#;
        let m: FlameManifest = serde_json::from_str(sample).unwrap();
        assert_eq!(m.overrides, "src");
        assert_eq!(m.name, "Custom");
        // 无 primary → 取第一个。
        let loader = m.minecraft.primary_loader().unwrap();
        assert_eq!(loader.split_id(), ("neoforge".to_string(), "47.1.0".to_string()));
    }

    #[test]
    fn wrong_manifest_type_is_invalid() {
        let sample = r#"{
            "manifestType": "somethingElse",
            "manifestVersion": 2,
            "minecraft": { "version": "1.20.1" }
        }"#;
        let m: FlameManifest = serde_json::from_str(sample).unwrap();
        assert!(!m.is_valid());
    }

    #[test]
    fn loader_id_without_dash_falls_back_to_family_only() {
        let l = FlameModLoader { id: "forge".to_string(), primary: false };
        assert_eq!(l.split_id(), ("forge".to_string(), String::new()));
    }
}
