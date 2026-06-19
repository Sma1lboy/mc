//! Technic 整合包(Solder API)结构(仅建模)。
//!
//! **非本地 zip**。两形态:
//! - (a) 单 zip:整包解压进 `.minecraft`,无 manifest,MC 版本来自 Platform API,loader 解压后嗅探。
//! - (b) Solder:`GET {solder}/modpack/{pack}/{version}` 得 [`TechnicSolderBuild`],每个
//!   `mods[].url` 是小 zip **按序**叠加(后盖前),loader 嗅探。
//!
//! 易错点(对照 `docs/modules/modpack-formats.md` §6):
//! - Solder 的 `mods[]` 是 zip(解压)而非 drop-in jar,且**按序**叠加。
//! - 只给 md5 → 统一模型走 md5 校验(`url` 任意 host)。
//! - loader 靠解压后嗅探(Solder build 本身不显式声明 loader)。
//!
//! 字段一律 `#[serde(default)]` 容错。

use serde::{Deserialize, Serialize};

/// Solder `GET /modpack/{pack}/{version}` 返回的一个 build。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TechnicSolderBuild {
    /// MC 版本。
    #[serde(default)]
    pub minecraft: String,
    /// 该 build 自身的版本(可选)。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// 推荐 Java 版本(部分 Solder 实例提供)。
    #[serde(rename = "java", default, skip_serializing_if = "Option::is_none")]
    pub java: Option<String>,
    /// 推荐内存(MB)。
    #[serde(rename = "memory", default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<String>,
    /// 按序叠加的 mod zip(后盖前)。
    #[serde(default)]
    pub mods: Vec<TechnicMod>,
}

/// Solder build 的一个 mod 包(小 zip,解压叠加)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TechnicMod {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// 该 zip 的 md5(`url` 任意 host → 强制校验)。
    #[serde(default)]
    pub md5: String,
    /// 小 zip 下载地址。
    #[serde(default)]
    pub url: String,
    /// 文件大小(字节),部分 Solder 提供。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
}

/// Platform API `GET /modpack/{slug}` 返回的包元信息(单 zip 形态用它取 MC 版本等)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TechnicPack {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "displayName", default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    /// 直接下载的整包 zip(单 zip 形态)。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Solder 实例根 url(存在则走 Solder 形态)。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub solder: String,
    /// 推荐 / 最新 build 版本。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub recommended: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub latest: String,
    /// 已知 build 列表。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub builds: Vec<String>,
    /// 包 logo。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<serde_json::Value>,
}

impl TechnicPack {
    /// 是否走 Solder 形态(有 `solder` 根 url)。否则是单 zip 直下。
    pub fn uses_solder(&self) -> bool {
        !self.solder.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_solder_build_ordered_mods() {
        let sample = r#"{
            "minecraft": "1.7.10",
            "version": "1.2.3",
            "java": "8",
            "memory": "4096",
            "mods": [
                { "name": "base", "version": "1.0", "md5": "aaa", "url": "https://x/base.zip" },
                { "name": "overlay", "version": "1.0", "md5": "bbb", "url": "https://x/overlay.zip", "filesize": 1024 }
            ]
        }"#;
        let build: TechnicSolderBuild = serde_json::from_str(sample).unwrap();
        assert_eq!(build.minecraft, "1.7.10");
        assert_eq!(build.version, "1.2.3");
        assert_eq!(build.java.as_deref(), Some("8"));
        assert_eq!(build.mods.len(), 2);
        // 顺序保留(叠加序)。
        assert_eq!(build.mods[0].name, "base");
        assert_eq!(build.mods[1].name, "overlay");
        assert_eq!(build.mods[0].md5, "aaa");
        assert_eq!(build.mods[1].filesize, Some(1024));
    }

    #[test]
    fn pack_solder_vs_single_zip() {
        let solder: TechnicPack = serde_json::from_str(
            r#"{ "name": "p", "solder": "https://solder.example/api", "recommended": "1.0" }"#,
        )
        .unwrap();
        assert!(solder.uses_solder());

        let single: TechnicPack =
            serde_json::from_str(r#"{ "name": "p", "url": "https://x/pack.zip" }"#).unwrap();
        assert!(!single.uses_solder());
        assert_eq!(single.url, "https://x/pack.zip");
    }
}
