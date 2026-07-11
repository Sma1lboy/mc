//! packwiz 格式(TOML,两级)。
//!
//! 不是 zip,是一棵 TOML 文件(常为 git 仓库 / HTTP)。三类文件:
//! - `pack.toml`   根清单:[`PackToml`] —— `name`、`versions{minecraft, fabric/forge/…}`、`index{file,hash-format,hash}`。
//! - `index.toml`  文件清单:[`IndexToml`] —— `hash-format` + `files[]{file,hash,metafile,preserve}`。
//! - `mods/<slug>.pw.toml`  单 mod 元文件:[`PwToml`] —— `name`、`filename`、`side`、`download{...}`、`update{modrinth|curseforge}`。
//!
//! 易错点(对照 `docs/modules/modpack-formats.md` §2):
//! - `metafile=false` 的 `index.toml` 条目是就地哈希的真实文件(config 等),不指向 `.pw.toml`。
//! - Prism 往 `.pw.toml` 注 `x-prismlauncher-*` 扩展键 → 读时必须可选(`#[serde(default)]` 容忍未知)。
//! - TOML 里 `hash-format` 是连字符键,serde 用 `rename` 对齐。
//!
//! **依赖约束**:本 crate **不引入** `toml` 依赖(见 `crates/mc-core/Cargo.toml`)。这些
//! 结构派生了 `Serialize`/`Deserialize`,可被任意 TOML (反)序列化器驱动;同时本模块自带
//! 一个**仅覆盖 packwiz 这一子集**的最小手写 TOML 读取器([`parse_pack_toml`] /
//! [`parse_index_toml`] / [`parse_pw_toml`]),无新依赖即可在导入路径里直接解析。该读取器
//! 不是通用 TOML 实现:它处理 packwiz 实际用到的形态(裸键值、`[table]`、`[[files]]`
//! 数组表、字符串 / 整数 / 布尔标量、行内 `# 注释`)。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

// ===========================================================================
// 结构(可被任意 TOML (反)序列化器驱动)
// ===========================================================================

/// `pack.toml` 根清单。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackToml {
    #[serde(default)]
    pub name: String,
    /// 可选包版本。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// 可选 pack 格式声明(`pack-format`,如 `packwiz:1.1.0`)。
    #[serde(default, rename = "pack-format", skip_serializing_if = "String::is_empty")]
    pub pack_format: String,
    /// `[versions]`:`minecraft` 必有,loader 键之一(fabric/forge/neoforge/quilt/liteloader)。
    #[serde(default)]
    pub versions: BTreeMap<String, String>,
    /// `[index]`:指向 `index.toml` 及其哈希。
    #[serde(default)]
    pub index: PackIndexRef,
}

/// `pack.toml` 里的 `[index]` 表。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackIndexRef {
    /// 相对路径,通常 `index.toml`。
    #[serde(default)]
    pub file: String,
    /// 哈希算法(`sha256` / `sha512` / …)。
    #[serde(default, rename = "hash-format")]
    pub hash_format: String,
    #[serde(default)]
    pub hash: String,
}

/// `index.toml` 文件清单。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexToml {
    /// 默认哈希算法(各 `files[]` 可不重复声明)。
    #[serde(default, rename = "hash-format")]
    pub hash_format: String,
    #[serde(default)]
    pub files: Vec<IndexFile>,
}

/// `index.toml` 的 `[[files]]` 条目。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexFile {
    pub file: String,
    #[serde(default)]
    pub hash: String,
    /// 可覆盖 index 级默认算法。
    #[serde(default, rename = "hash-format", skip_serializing_if = "String::is_empty")]
    pub hash_format: String,
    /// `true` → 指向 `mods/<slug>.pw.toml`(元文件);`false` → 就地哈希的真实文件。
    #[serde(default)]
    pub metafile: bool,
    /// 升级时是否保留本地修改。
    #[serde(default)]
    pub preserve: bool,
}

/// `mods/<slug>.pw.toml` 单 mod 元文件。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PwToml {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub filename: String,
    /// `both` / `client` / `server`。
    #[serde(default)]
    pub side: String,
    #[serde(default)]
    pub download: PwDownload,
    /// `[update.modrinth]` / `[update.curseforge]`(可空:纯 URL mod 无 update 源)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<PwUpdate>,
}

/// `.pw.toml` 的 `[download]` 表。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PwDownload {
    /// `url`(直链)或经 `mode` 间接解析。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// `url`(默认)/ `metadata:curseforge` 等。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(default, rename = "hash-format")]
    pub hash_format: String,
    #[serde(default)]
    pub hash: String,
}

/// `.pw.toml` 的 `[update]` 表(两个 provider 子表二选一)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PwUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modrinth: Option<PwUpdateModrinth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curseforge: Option<PwUpdateCurseForge>,
}

/// `[update.modrinth]`:`mod-id` + `version`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PwUpdateModrinth {
    #[serde(default, rename = "mod-id")]
    pub mod_id: String,
    #[serde(default)]
    pub version: String,
}

/// `[update.curseforge]`:`file-id` + `project-id`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PwUpdateCurseForge {
    #[serde(default, rename = "file-id")]
    pub file_id: i64,
    #[serde(default, rename = "project-id")]
    pub project_id: i64,
}

// ===========================================================================
// 最小手写 TOML 读取器(仅覆盖 packwiz 子集,无外部依赖)
// ===========================================================================

/// 一个 TOML 标量值(packwiz 用到的:字符串 / 整数 / 布尔)。
#[derive(Debug, Clone, PartialEq)]
enum TomlScalar {
    Str(String),
    Int(i64),
    Bool(bool),
}

impl TomlScalar {
    fn as_str(&self) -> Option<&str> {
        match self {
            TomlScalar::Str(s) => Some(s),
            _ => None,
        }
    }
    fn as_int(&self) -> Option<i64> {
        match self {
            TomlScalar::Int(i) => Some(*i),
            _ => None,
        }
    }
    fn as_bool(&self) -> Option<bool> {
        match self {
            TomlScalar::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// 一个解析出的表(`[table]` 或顶层),按出现序保留键。
#[derive(Debug, Default)]
struct TomlTable {
    keys: BTreeMap<String, TomlScalar>,
}

/// 极简 TOML 文档:命名表 + `[[file]]` 风格数组表。仅服务 packwiz 子集。
#[derive(Debug, Default)]
struct TomlDoc {
    /// 顶层(无 `[header]` 前的)键。
    root: TomlTable,
    /// 命名表:`[a]` / `[a.b]` → 全限定名 → 表。
    tables: BTreeMap<String, TomlTable>,
    /// 数组表:`[[files]]` → header → 多个表(按序)。
    arrays: BTreeMap<String, Vec<TomlTable>>,
}

/// 解析一个标量右值(去掉行内注释、引号、识别 true/false/整数)。
fn parse_scalar(raw: &str) -> Option<TomlScalar> {
    let stripped = strip_inline_comment(raw);
    let v = stripped.trim();
    if v.is_empty() {
        return None;
    }
    // 带引号字符串(单 / 双引号)。
    if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
        || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
    {
        let inner = &v[1..v.len() - 1];
        // 仅处理双引号里的最常见转义(\" \\ \n \t);packwiz 字段几无复杂转义。
        let unescaped = if v.starts_with('"') {
            inner
                .replace("\\\"", "\"")
                .replace("\\n", "\n")
                .replace("\\t", "\t")
                .replace("\\\\", "\\")
        } else {
            inner.to_string()
        };
        return Some(TomlScalar::Str(unescaped));
    }
    if v == "true" {
        return Some(TomlScalar::Bool(true));
    }
    if v == "false" {
        return Some(TomlScalar::Bool(false));
    }
    if let Ok(i) = v.parse::<i64>() {
        return Some(TomlScalar::Int(i));
    }
    // 兜底:当裸字符串(packwiz 偶有不加引号的简单值)。
    Some(TomlScalar::Str(v.to_string()))
}

/// 去掉一行里引号外的 `#` 行内注释。
fn strip_inline_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut in_str: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match in_str {
            Some(q) => {
                out.push(c);
                if c == '\\' {
                    // 跳过被转义的下一个字符。
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                } else if c == q {
                    in_str = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    in_str = Some(c);
                    out.push(c);
                } else if c == '#' {
                    break;
                } else {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// 把 TOML 文本解析成 [`TomlDoc`]。只识别 packwiz 用到的形态。
fn parse_toml(text: &str) -> TomlDoc {
    let mut doc = TomlDoc::default();
    // 当前写入目标:None=root;Some((name,is_array_last))。
    enum Target {
        Root,
        Table(String),
        Array(String),
    }
    let mut target = Target::Root;

    for raw_line in text.lines() {
        let line = strip_inline_comment(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // 数组表头 [[name]]。
        if let Some(inner) = trimmed.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
            let name = inner.trim().to_string();
            doc.arrays.entry(name.clone()).or_default().push(TomlTable::default());
            target = Target::Array(name);
            continue;
        }
        // 普通表头 [name]。
        if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let name = inner.trim().to_string();
            doc.tables.entry(name.clone()).or_default();
            target = Target::Table(name);
            continue;
        }
        // 键值。
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim().to_string();
        let Some(val) = parse_scalar(v) else { continue };
        let tbl = match &target {
            Target::Root => &mut doc.root,
            Target::Table(name) => doc.tables.entry(name.clone()).or_default(),
            Target::Array(name) => doc
                .arrays
                .get_mut(name)
                .and_then(|v| v.last_mut())
                .expect("array target always has a pushed table"),
        };
        tbl.keys.insert(key, val);
    }
    doc
}

// ----- 子集 → 强类型映射 -----

fn map_scalars_to_string_map(tbl: &TomlTable) -> BTreeMap<String, String> {
    tbl.keys
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

/// 解析 `pack.toml`。
pub fn parse_pack_toml(text: &str) -> PackToml {
    let doc = parse_toml(text);
    let s = |k: &str| doc.root.keys.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();

    let versions = doc
        .tables
        .get("versions")
        .map(map_scalars_to_string_map)
        .unwrap_or_default();

    let index = doc
        .tables
        .get("index")
        .map(|t| PackIndexRef {
            file: t.keys.get("file").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            hash_format: t
                .keys
                .get("hash-format")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            hash: t.keys.get("hash").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        })
        .unwrap_or_default();

    PackToml {
        name: s("name"),
        version: s("version"),
        pack_format: s("pack-format"),
        versions,
        index,
    }
}

/// 解析 `index.toml`。
pub fn parse_index_toml(text: &str) -> IndexToml {
    let doc = parse_toml(text);
    let hash_format = doc
        .root
        .keys
        .get("hash-format")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let files = doc
        .arrays
        .get("files")
        .map(|rows| {
            rows.iter()
                .map(|t| IndexFile {
                    file: t.keys.get("file").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    hash: t.keys.get("hash").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    hash_format: t
                        .keys
                        .get("hash-format")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    metafile: t.keys.get("metafile").and_then(|v| v.as_bool()).unwrap_or(false),
                    preserve: t.keys.get("preserve").and_then(|v| v.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();

    IndexToml { hash_format, files }
}

/// 解析 `mods/<slug>.pw.toml`。`x-prismlauncher-*` 等扩展键被忽略(不影响已建模字段)。
pub fn parse_pw_toml(text: &str) -> PwToml {
    let doc = parse_toml(text);
    let s = |k: &str| doc.root.keys.get(k).and_then(|v| v.as_str()).unwrap_or_default().to_string();

    let download = doc
        .tables
        .get("download")
        .map(|t| PwDownload {
            url: t.keys.get("url").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            mode: t.keys.get("mode").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
            hash_format: t
                .keys
                .get("hash-format")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
            hash: t.keys.get("hash").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        })
        .unwrap_or_default();

    let modrinth = doc.tables.get("update.modrinth").map(|t| PwUpdateModrinth {
        mod_id: t.keys.get("mod-id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
        version: t.keys.get("version").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
    });
    let curseforge = doc.tables.get("update.curseforge").map(|t| PwUpdateCurseForge {
        file_id: t.keys.get("file-id").and_then(|v| v.as_int()).unwrap_or(0),
        project_id: t.keys.get("project-id").and_then(|v| v.as_int()).unwrap_or(0),
    });
    let update = if modrinth.is_some() || curseforge.is_some() {
        Some(PwUpdate { modrinth, curseforge })
    } else {
        None
    };

    PwToml {
        name: s("name"),
        filename: s("filename"),
        side: s("side"),
        download,
        update,
    }
}

#[cfg(test)]
mod tests;
