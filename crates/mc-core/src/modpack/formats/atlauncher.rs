//! ATLauncher 远程整合包(`Configs.json`)结构(仅建模)。
//!
//! **非本地 zip**:用户从平台选包,`safeName`(去非字母数字)→ `GET .../Configs.json`
//! 得 [`AtlPackVersion`]。mod `download` ∈ `server|direct|browser`(browser=blocked 手动);
//! `type` 路由目标目录;`extractTo`/`decompType` 解压;md5 校验;`client==false` 跳过。
//! `Configs.zip` 是 overrides 包。
//!
//! 易错点(对照 `docs/modules/modpack-formats.md` §6):
//! - `download=="browser"` → blocked,绝不伪造 URL,走手动下载流。
//! - `url` 任意 host → **强制** md5 校验(ATL 给 md5)。
//! - `extractFolder`(`%s%` 分隔)是攻击面,落盘前必过 `safe_join`。
//! - `client==false` 的 mod 在客户端导入时跳过。
//!
//! 字段一律 `#[serde(default)]` 容错:平台 json 字段缺失常见,不让单字段缺失打挂。

use serde::{Deserialize, Serialize};

/// `Configs.json` 顶层:一个 ATLauncher 包的某个版本。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AtlPackVersion {
    /// 包版本号。
    #[serde(default)]
    pub version: String,
    /// MC 版本。
    #[serde(default)]
    pub minecraft: String,
    /// loader 声明(可空:纯原版包)。
    #[serde(default)]
    pub loader: Option<AtlLoader>,
    /// mod 列表。
    #[serde(default)]
    pub mods: Vec<AtlMod>,
    /// `[configs]`:overrides 包(`Configs.zip`)的 sha1。
    #[serde(default)]
    pub configs: Option<AtlConfigs>,
    /// 自定义主类(覆盖 loader 推断)。
    #[serde(rename = "mainClass", default, skip_serializing_if = "Option::is_none")]
    pub main_class: Option<String>,
    /// 追加启动参数。
    #[serde(rename = "extraArguments", default, skip_serializing_if = "Option::is_none")]
    pub extra_arguments: Option<String>,
    /// 升级时保留的文件 glob。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keeps: Vec<String>,
    /// 升级时删除的文件 glob。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deletes: Vec<String>,
}

/// `loader` 子对象:`type`(forge/fabric/…)+ `metadata`(版本等,形态随 type 而异)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AtlLoader {
    /// loader 家族(`forge` / `fabric` / `quilt` / `neoforge` / `legacyfabric` …)。
    #[serde(rename = "type", default)]
    pub kind: String,
    /// 与 type 绑定的元数据(版本等),形态各异 → 用 `Value` 承接。
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// `mods[]` 中的一项。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AtlMod {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// 落盘文件名。
    #[serde(default)]
    pub file: String,
    /// `url` 下载地址(`download=="browser"` 时常缺/为站点页)。
    #[serde(default)]
    pub url: String,
    /// 下载方式:`server` / `direct` / `browser`(browser=blocked 手动)。
    #[serde(default)]
    pub download: String,
    /// 资源类型,路由目标目录(`mods` / `resourcepack` / `jar` / `dependency` …)。
    #[serde(rename = "type", default)]
    pub kind: String,
    /// md5 校验和(ATL 给 md5;url 任意 host 故强制校验)。
    #[serde(default)]
    pub md5: String,
    /// 解压目标(`%s%` 分隔的相对路径);落盘前必过 `safe_join`。
    #[serde(rename = "extractTo", default, skip_serializing_if = "Option::is_none")]
    pub extract_to: Option<String>,
    /// 解压文件夹名(可选)。
    #[serde(rename = "extractFolder", default, skip_serializing_if = "Option::is_none")]
    pub extract_folder: Option<String>,
    /// 解压算法(`zip` / `gzip` …);存在则该 mod 是压缩包,解压而非直落。
    #[serde(rename = "decompType", default, skip_serializing_if = "Option::is_none")]
    pub decomp_type: Option<String>,
    /// 解压源文件(配合 decompType)。
    #[serde(rename = "decompFile", default, skip_serializing_if = "Option::is_none")]
    pub decomp_file: Option<String>,
    /// 是否客户端需要(`false` → 客户端导入跳过)。默认 true。
    #[serde(default = "default_true")]
    pub client: bool,
    /// 是否服务端需要。默认 true(客户端导入不关心)。
    #[serde(default = "default_true")]
    pub server: bool,
    /// 是否可选(optional mod)。默认 false。
    #[serde(default)]
    pub optional: bool,
}

fn default_true() -> bool {
    true
}

/// `configs` 子对象:overrides 包(`Configs.zip`)的引用。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AtlConfigs {
    /// `Configs.zip` 的 sha1。
    #[serde(default)]
    pub sha1: String,
    /// 文件大小(字节)。
    #[serde(default)]
    pub filesize: Option<u64>,
}

impl AtlMod {
    /// 该 mod 是否被作者禁止第三方分发(`download=="browser"` → blocked,走手动流)。
    pub fn is_blocked(&self) -> bool {
        self.download.eq_ignore_ascii_case("browser")
    }

    /// 该 mod 是否需要解压(有 `decompType`)。
    pub fn is_archive(&self) -> bool {
        self.decomp_type.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pack_version_with_loader_and_mods() {
        let sample = r#"{
            "version": "1.5.0",
            "minecraft": "1.20.1",
            "loader": { "type": "fabric", "metadata": { "version": "0.15.7" } },
            "mainClass": "com.example.Main",
            "extraArguments": "--tweakClass foo",
            "configs": { "sha1": "abc123", "filesize": 4096 },
            "keeps": ["options.txt"],
            "deletes": ["mods/old.jar"],
            "mods": [
                {
                    "name": "Sodium", "version": "0.5.3", "file": "sodium.jar",
                    "url": "https://cdn.example/sodium.jar", "download": "direct",
                    "type": "mods", "md5": "deadbeef", "client": true, "server": false
                },
                {
                    "name": "Blocked Mod", "version": "1.0", "file": "blocked.jar",
                    "url": "https://www.curseforge.com/...", "download": "browser",
                    "type": "mods", "md5": "cafebabe"
                }
            ]
        }"#;
        let pv: AtlPackVersion = serde_json::from_str(sample).unwrap();
        assert_eq!(pv.minecraft, "1.20.1");
        assert_eq!(pv.loader.as_ref().unwrap().kind, "fabric");
        assert_eq!(pv.main_class.as_deref(), Some("com.example.Main"));
        assert_eq!(pv.configs.as_ref().unwrap().sha1, "abc123");
        assert_eq!(pv.keeps, vec!["options.txt".to_string()]);

        assert_eq!(pv.mods.len(), 2);
        let sodium = &pv.mods[0];
        assert_eq!(sodium.download, "direct");
        assert_eq!(sodium.md5, "deadbeef");
        assert!(sodium.client);
        assert!(!sodium.server);
        assert!(!sodium.is_blocked());

        let blocked = &pv.mods[1];
        assert!(blocked.is_blocked(), "download==browser 应判为 blocked");
        // 默认 client/server true(未给)。
        assert!(blocked.client);
        assert!(blocked.server);
    }

    #[test]
    fn archive_mod_with_decomp_and_extract() {
        let sample = r#"{
            "name": "Configs", "file": "configs.zip", "url": "https://x/configs.zip",
            "download": "server", "type": "extract", "md5": "1111",
            "decompType": "zip", "decompFile": "configs.zip", "extractTo": "config%s%sub"
        }"#;
        let m: AtlMod = serde_json::from_str(sample).unwrap();
        assert!(m.is_archive());
        assert_eq!(m.decomp_type.as_deref(), Some("zip"));
        assert_eq!(m.extract_to.as_deref(), Some("config%s%sub"));
    }

    #[test]
    fn vanilla_pack_has_no_loader() {
        let sample = r#"{ "version": "1.0", "minecraft": "1.20.1" }"#;
        let pv: AtlPackVersion = serde_json::from_str(sample).unwrap();
        assert!(pv.loader.is_none());
        assert!(pv.mods.is_empty());
    }
}
