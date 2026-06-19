//! Strongly-typed Mojang version json. Supports both the 1.13+ `arguments`
//! object and the pre-1.13 `minecraftArguments` string, plus `inheritsFrom`.

use serde::Deserialize;

use super::library::{Artifact, Library};
use super::rule::Rule;

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default, rename = "totalSize")]
    pub total_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Downloads {
    #[serde(default)]
    pub client: Option<Artifact>,
    #[serde(default)]
    pub server: Option<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaVersionReq {
    #[serde(default)]
    pub component: Option<String>,
    #[serde(rename = "majorVersion")]
    pub major_version: u8,
}

/// One element of an `arguments.game` / `arguments.jvm` array: either a bare
/// string or a conditional `{rules, value}` object.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional {
        rules: Vec<Rule>,
        value: StringOrList,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    One(String),
    Many(Vec<String>),
}

impl StringOrList {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            StringOrList::One(s) => vec![s],
            StringOrList::Many(v) => v,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingFile {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingEntry {
    pub argument: String,
    pub file: LoggingFile,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Logging {
    #[serde(default)]
    pub client: Option<LoggingEntry>,
}

/// The raw, single-file version json exactly as Mojang / loaders ship it.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionJson {
    pub id: String,
    #[serde(default, rename = "inheritsFrom")]
    pub inherits_from: Option<String>,
    #[serde(default, rename = "mainClass")]
    pub main_class: Option<String>,

    #[serde(default)]
    pub libraries: Vec<Library>,

    // 1.13+ structured arguments, mutually exclusive with `minecraft_arguments`.
    #[serde(default)]
    pub arguments: Option<Arguments>,
    // pre-1.13 single string.
    #[serde(default, rename = "minecraftArguments")]
    pub minecraft_arguments: Option<String>,

    #[serde(default, rename = "assetIndex")]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub assets: Option<String>,

    #[serde(default)]
    pub downloads: Downloads,

    #[serde(default, rename = "javaVersion")]
    pub java_version: Option<JavaVersionReq>,

    #[serde(default, rename = "type")]
    pub release_type: Option<String>,

    #[serde(default)]
    pub logging: Logging,
}

impl VersionJson {
    pub fn parse(s: &str) -> Result<VersionJson, serde_json::Error> {
        serde_json::from_str(s)
    }
}
