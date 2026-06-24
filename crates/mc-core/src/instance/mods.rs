//! 本地 Mod 管理 —— 扫描实例 `mods/` 目录、读取每个 jar 的元数据、启停与删除。
//!
//! 设计要点:
//! - 一个 mod 文件可能处于"启用"或"停用"两态。沿用 MultiMC/Prism/HMCL 的约定:
//!   停用的 mod 文件名末尾追加 `.disabled`(即 `foo.jar` ↔ `foo.jar.disabled`),
//!   loader 只加载 `*.jar`,从而无需移动文件即可启停。
//! - 元数据来源各 loader 不同:
//!   · Fabric:`fabric.mod.json`(JSON)
//!   · Quilt :`quilt.mod.json`(JSON,字段在 `quilt_loader` 下)
//!   · Forge / NeoForge:`META-INF/mods.toml` / `META-INF/neoforge.mods.toml`(TOML 文本)
//!   为不引入额外依赖,Forge 系的 TOML 用一个**轻量、容错**的手写解析器提取所需字段。
//! - **健壮性优先**:解析单个 jar 出任何错误(打不开、不是 zip、字段缺失、JSON 损坏)
//!   都不得 panic,也不能中断对其它 jar 的扫描 —— 该 jar 退化为"仅文件名"信息。
//! - `ModInfo` 实现 `Serialize` 以便直接经 IPC/HTTP 输出给上层 UI。

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoResultExt, Result};
use crate::instance::Instance;

/// 单个本地 mod 的元数据视图。字段尽量贴近 UI 列表展示所需。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ModInfo {
    /// 磁盘上的实际文件名(含 `.disabled` 后缀,如有)。用作启停/删除的稳定标识。
    pub file_name: String,
    /// 是否启用 = 文件名不以 `.disabled` 结尾。
    pub enabled: bool,
    /// 展示名;读不到元数据时回退为去掉扩展名的文件名。
    pub name: String,
    /// 版本号(部分 mod 用占位符如 `${version}`,原样保留)。
    pub version: Option<String>,
    /// mod 内部 id(fabric `id` / quilt `id` / forge `modId`)。
    pub mod_id: Option<String>,
    /// loader 家族:`fabric` / `quilt` / `forge` / `neoforge` / `unknown`。
    pub loader: String,
    /// 作者列表(各 loader 格式差异较大,这里统一拍平成字符串数组)。
    pub authors: Vec<String>,
    /// 描述(通常为一句话)。
    pub description: Option<String>,
    /// 文件字节大小。
    pub size: u64,
}

/// 停用文件的统一后缀。
const DISABLED_SUFFIX: &str = ".disabled";

/// 列出实例 `mods/` 目录下的所有 mod(包含已停用的)。
///
/// 扫描规则:接受文件名以 `.jar` 或 `.jar.disabled` 结尾的普通文件。
/// 对每个文件尝试读取元数据;读取/解析失败的文件会退化为"仅文件名"条目,
/// **不会**被丢弃也**不会** panic。结果按 `name` 不区分大小写排序,保证展示顺序稳定。
///
/// 若 `mods/` 目录不存在(实例从未装过 mod),返回空列表。
pub fn list_mods(inst: &Instance) -> Vec<ModInfo> {
    let mods_dir = inst.mods_dir();

    let entries = match std::fs::read_dir(&mods_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<ModInfo> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue, // 非 UTF-8 文件名,跳过(无法稳定标识)。
        };

        // 只接受 *.jar 与 *.jar.disabled。
        let enabled = if file_name.ends_with(DISABLED_SUFFIX) {
            // 去掉 .disabled 后必须仍是 .jar 才算 mod 文件(排除 foo.txt.disabled 之类)。
            if !strip_disabled(&file_name).ends_with(".jar") {
                continue;
            }
            false
        } else if file_name.ends_with(".jar") {
            true
        } else {
            continue;
        };

        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        // 解析元数据;任何失败都退化为基于文件名的兜底信息。
        let mut info = read_mod_metadata(&path).unwrap_or_else(|| fallback_info(&file_name));
        info.file_name = file_name;
        info.enabled = enabled;
        info.size = size;

        out.push(info);
    }

    // 不区分大小写按展示名排序;同名再按文件名兜底,保证确定性。
    out.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    out
}

/// 启用/停用一个 mod:在 `<name>.jar` 与 `<name>.jar.disabled` 之间重命名。
///
/// `file_name` 为 [`list_mods`] 返回的当前文件名(可带或不带 `.disabled`)。
/// 若目标态已满足(已是想要的后缀),则为 no-op,直接返回 `Ok`。
pub fn set_mod_enabled(inst: &Instance, file_name: &str, enabled: bool) -> Result<()> {
    // file_name 来自前端,校验为单一路径段防穿越(rename 出 mods/ 会破坏其它目录)。
    if !crate::fs::is_safe_segment(file_name) {
        return Err(crate::error::CoreError::other(format!("非法 mod 文件名: {file_name}")));
    }
    let mods_dir = inst.mods_dir();

    // 以"去掉 .disabled 的基名"为锚,推导当前路径与目标路径,避免依赖传入后缀是否正确。
    let base = strip_disabled(file_name).to_string();
    let enabled_path = mods_dir.join(&base);
    let disabled_path = mods_dir.join(format!("{base}{DISABLED_SUFFIX}"));

    let (src, dst) = if enabled {
        (disabled_path, enabled_path)
    } else {
        (enabled_path, disabled_path)
    };

    // 已处于目标态:目标存在且源不存在 → 无需操作。
    if dst.exists() && !src.exists() {
        return Ok(());
    }

    std::fs::rename(&src, &dst).with_path(&src)
}

/// 删除一个 mod 文件。优先移入系统回收站(可恢复);若回收站不可用(如某些 Linux
/// 无 trash 环境、CI),回退到不可逆的 [`std::fs::remove_file`]。
///
/// `file_name` 可带或不带 `.disabled`;实际删除磁盘上存在的那一个。
pub fn delete_mod(inst: &Instance, file_name: &str) -> Result<()> {
    // file_name 来自前端,校验为单一路径段防穿越(删除可逃出 mods/ 误删其它文件)。
    if !crate::fs::is_safe_segment(file_name) {
        return Err(crate::error::CoreError::other(format!("非法 mod 文件名: {file_name}")));
    }
    let mods_dir = inst.mods_dir();

    // 定位真实存在的文件:传入名 → 基名(.jar) → 停用名(.jar.disabled)。
    let base = strip_disabled(file_name).to_string();
    let candidates = [
        mods_dir.join(file_name),
        mods_dir.join(&base),
        mods_dir.join(format!("{base}{DISABLED_SUFFIX}")),
    ];

    let target = candidates.into_iter().find(|p| p.exists());
    let target = match target {
        Some(p) => p,
        // 文件已不存在:视作删除成功(幂等),避免上层因竞态报错。
        None => return Ok(()),
    };

    // trash::delete 失败(平台不支持/无回收站)时回退硬删除。
    if trash::delete(&target).is_err() {
        std::fs::remove_file(&target).with_path(&target)?;
    }
    Ok(())
}

/// 读取单个 jar 的内部 `mod_id`(读不出 / 无该字段时返回 `None`)。
/// 用于「装新版自动清掉同一 mod 的旧 jar」时按 mod_id 配对。
pub fn read_mod_id(path: &Path) -> Option<String> {
    read_mod_metadata(path).and_then(|m| m.mod_id)
}

/// 装好 `keep_file_name` 这个新版本后,清理 `mods/` 里**同一个 mod(相同 `mod_id`)
/// 但文件名不同**的旧 jar —— 否则两个版本会声明同一 `modId`,导致游戏启动直接崩溃
/// (duplicate mod id)。返回被清理掉的旧文件名列表。
///
/// 安全约束:读不出新文件的 `mod_id` 时**不删除任何东西**(无法可靠判定归属);
/// 旧文件优先移入回收站(可找回),回收站不可用才硬删。已停用(`.disabled`)的同
/// mod 旧版同样清理 —— 留着它也会在用户重新启用时再次冲突。
pub fn remove_superseded(inst: &Instance, keep_file_name: &str) -> Result<Vec<String>> {
    let mods_dir = inst.mods_dir();

    let keep_id = match read_mod_id(&mods_dir.join(keep_file_name)) {
        Some(id) => id,
        None => return Ok(Vec::new()),
    };

    let entries = match std::fs::read_dir(&mods_dir) {
        Ok(e) => e,
        Err(_) => return Ok(Vec::new()),
    };

    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if name == keep_file_name {
            continue;
        }
        // 只看 mod jar(含 .disabled),其它文件忽略。
        let is_mod_jar = name.ends_with(".jar")
            || (name.ends_with(DISABLED_SUFFIX) && strip_disabled(&name).ends_with(".jar"));
        if !is_mod_jar {
            continue;
        }
        if read_mod_id(&path).as_deref() == Some(keep_id.as_str()) {
            if trash::delete(&path).is_err() {
                std::fs::remove_file(&path).with_path(&path)?;
            }
            removed.push(name);
        }
    }
    Ok(removed)
}

/// 把一个本地 `.jar` 拖拽导入实例 `mods/` 目录,返回落盘文件名。
///
/// 校验:源必须存在、文件名是 `.jar`(忽略大小写)。重名直接覆盖——用户主动拖入即视为
/// 替换意图。文件名取源的 basename(不含路径分量),天然不含路径穿越。
pub fn import_local_mod(inst: &Instance, source: &Path) -> Result<String> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CoreError::other("无效的文件名"))?
        .to_string();
    if !name.to_ascii_lowercase().ends_with(".jar") {
        return Err(CoreError::other("只支持拖入 .jar mod 文件"));
    }
    let dir = inst.mods_dir();
    std::fs::create_dir_all(&dir).with_path(&dir)?;
    std::fs::copy(source, dir.join(&name)).with_path(source)?;
    Ok(name)
}

// ───────────────────────── 内部:文件名与兜底 ─────────────────────────

/// 去掉末尾的 `.disabled`(若有),得到"启用态"基名。
fn strip_disabled(name: &str) -> &str {
    name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name)
}

/// 由文件名构造兜底元数据:name = 去掉 `.jar`/`.jar.disabled` 的名字,loader = unknown。
fn fallback_info(file_name: &str) -> ModInfo {
    let base = strip_disabled(file_name);
    let name = base.strip_suffix(".jar").unwrap_or(base).to_string();
    ModInfo {
        file_name: file_name.to_string(),
        enabled: !file_name.ends_with(DISABLED_SUFFIX),
        name,
        version: None,
        mod_id: None,
        loader: "unknown".to_string(),
        authors: Vec::new(),
        description: None,
        size: 0,
    }
}

// ───────────────────────── 内部:jar 元数据读取 ─────────────────────────

/// 尝试从 jar 中读取元数据。按 fabric → quilt → forge/neoforge 顺序探测;
/// 全部缺失或解析失败时返回 `None`(交由调用方兜底)。
///
/// `file_name`/`enabled`/`size` 由调用方覆盖填充,这里只产出元数据相关字段。
fn read_mod_metadata(path: &Path) -> Option<ModInfo> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    // Fabric。
    if let Some(text) = read_entry(&mut archive, "fabric.mod.json") {
        if let Some(info) = parse_fabric(&text) {
            return Some(info);
        }
    }
    // Quilt。
    if let Some(text) = read_entry(&mut archive, "quilt.mod.json") {
        if let Some(info) = parse_quilt(&text) {
            return Some(info);
        }
    }
    // NeoForge(新)优先于 Forge:某些 jar 两者皆备,以更具体的 neoforge 为准。
    if let Some(text) = read_entry(&mut archive, "META-INF/neoforge.mods.toml") {
        if let Some(info) = parse_forge_toml(&text, "neoforge") {
            return Some(info);
        }
    }
    if let Some(text) = read_entry(&mut archive, "META-INF/mods.toml") {
        if let Some(info) = parse_forge_toml(&text, "forge") {
            return Some(info);
        }
    }

    None
}

/// 读取 zip 中指定条目为 UTF-8 文本;不存在或读取失败返回 `None`。
fn read_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    let mut buf = String::new();
    // 用 read_to_string,非 UTF-8 内容会失败 → None(容错跳过)。
    entry.read_to_string(&mut buf).ok()?;
    Some(buf)
}

// ───────────────────────── 内部:Fabric / Quilt JSON ─────────────────────────

/// 解析 `fabric.mod.json`。`authors` 可能是字符串数组,或对象数组(含 `name` 字段);
/// 两种都要兼容。`description` 可缺省。
fn parse_fabric(text: &str) -> Option<ModInfo> {
    #[derive(Deserialize)]
    struct Fabric {
        id: Option<String>,
        name: Option<String>,
        version: Option<String>,
        #[serde(default)]
        authors: Vec<serde_json::Value>,
        description: Option<String>,
    }

    let v: Fabric = serde_json::from_str(text).ok()?;
    let authors = v.authors.iter().filter_map(author_to_string).collect();
    let name = v
        .name
        .clone()
        .or_else(|| v.id.clone())
        .unwrap_or_else(|| "unknown".to_string());

    Some(ModInfo {
        file_name: String::new(),
        enabled: true,
        name,
        version: v.version,
        mod_id: v.id,
        loader: "fabric".to_string(),
        authors,
        description: v.description,
        size: 0,
    })
}

/// 解析 `quilt.mod.json`。核心字段在 `quilt_loader` 下,展示名/描述/作者在
/// `quilt_loader.metadata` 下。结构较深,逐层 `Option` 容错。
fn parse_quilt(text: &str) -> Option<ModInfo> {
    #[derive(Deserialize)]
    struct Quilt {
        quilt_loader: Option<QuiltLoader>,
    }
    #[derive(Deserialize)]
    struct QuiltLoader {
        id: Option<String>,
        version: Option<String>,
        metadata: Option<QuiltMeta>,
    }
    #[derive(Deserialize)]
    struct QuiltMeta {
        name: Option<String>,
        description: Option<String>,
        // contributors 通常是 { "Name": "Role", ... } 形式的对象;取其 key 作为作者名。
        contributors: Option<serde_json::Value>,
    }

    let v: Quilt = serde_json::from_str(text).ok()?;
    let loader = v.quilt_loader?;
    let meta = loader.metadata;

    let id = loader.id.clone();
    let name = meta
        .as_ref()
        .and_then(|m| m.name.clone())
        .or_else(|| id.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let description = meta.as_ref().and_then(|m| m.description.clone());
    let authors = meta
        .as_ref()
        .and_then(|m| m.contributors.as_ref())
        .map(contributors_to_authors)
        .unwrap_or_default();

    Some(ModInfo {
        file_name: String::new(),
        enabled: true,
        name,
        version: loader.version,
        mod_id: id,
        loader: "quilt".to_string(),
        authors,
        description,
        size: 0,
    })
}

/// 把 fabric `authors` 数组里的一项转成字符串:字符串原样,对象取 `name` 字段。
fn author_to_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => map
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// quilt `contributors`:若为对象取所有 key(贡献者名);若为数组取每个字符串项。
fn contributors_to_authors(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::Object(map) => map.keys().cloned().collect(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect(),
        serde_json::Value::String(s) => vec![s.clone()],
        _ => Vec::new(),
    }
}

// ───────────────────────── 内部:Forge / NeoForge TOML ─────────────────────────

/// 轻量手写解析 `mods.toml` / `neoforge.mods.toml`,**不引入 toml crate**。
///
/// 该文件结构稳定:顶层有若干 `key = "value"`,mod 信息在 `[[mods]]` 表数组的
/// 第一个表内(`modId` / `displayName` / `version` / `authors` / `description`)。
/// 我们只需要第一个 `[[mods]]`(单 jar 通常只声明一个 mod),并允许如下容错:
///   - 忽略注释(`#` 开头)与空行;
///   - 值可带或不带引号,支持 TOML 多行字符串 `'''...'''` / `"""..."""`(description 常用);
///   - 顶层的 `authors`(全局作者)在 `[[mods]]` 未声明 authors 时作为回退。
///
/// 这是"够用且容错"的解析,不追求完整 TOML 兼容性(超出本启动器需求且易引入 bug)。
fn parse_forge_toml(text: &str, loader: &str) -> Option<ModInfo> {
    // 顶层(任意 [[mods]] 之前)的 authors,作为 mod 块缺省时的回退。
    let mut top_authors: Option<String> = None;
    // 第一个 [[mods]] 块内的字段。
    let mut mod_id: Option<String> = None;
    let mut display_name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut mod_authors: Option<String> = None;
    let mut description: Option<String> = None;

    // 状态机:section 表示当前所在的表;只关心 "" (顶层) 与 "mods" (第一个 [[mods]])。
    #[derive(PartialEq)]
    enum Section {
        Top,
        FirstMods,
        Other, // 第二个 [[mods]] 或其它无关表,忽略。
    }
    let mut section = Section::Top;
    let mut seen_mods_table = false;

    let mut lines = text.lines().peekable();
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // 表头切换。
        if line.starts_with('[') {
            if line.starts_with("[[mods]]") {
                if seen_mods_table {
                    section = Section::Other; // 已处理过第一个 mods 块,后续忽略。
                } else {
                    seen_mods_table = true;
                    section = Section::FirstMods;
                }
            } else {
                // 任意其它表(如 [[dependencies.xxx]]),离开 mods 块。
                section = if seen_mods_table { Section::Other } else { Section::Top };
            }
            continue;
        }

        // 只处理 key = value 行(顶层或第一个 mods 块内)。
        if section == Section::Other {
            continue;
        }

        let (key, val) = match split_kv(line) {
            Some(kv) => kv,
            None => continue,
        };

        // 读取值:可能是多行字符串,需要把后续行也吞进来。
        let value = read_toml_value(val, &mut lines);

        match section {
            Section::Top => {
                if key == "authors" {
                    top_authors = Some(value);
                }
            }
            Section::FirstMods => match key {
                "modId" => mod_id = Some(value),
                "displayName" => display_name = Some(value),
                "version" => version = Some(value),
                "authors" => mod_authors = Some(value),
                "description" => description = Some(value),
                _ => {}
            },
            Section::Other => {}
        }
    }

    // 必须至少拿到 modId,否则视为不可识别(返回 None 让上层兜底)。
    let mod_id = mod_id?;

    let name = display_name.unwrap_or_else(|| mod_id.clone());
    let authors_raw = mod_authors.or(top_authors).unwrap_or_default();
    let authors = split_authors(&authors_raw);

    // 过滤掉 Forge 模板常见的占位符 `${file.jarVersion}` 等(以 `${` 开头),展示更干净。
    let version = version.filter(|v| !v.is_empty());
    let description = description.filter(|d| !d.trim().is_empty());

    Some(ModInfo {
        file_name: String::new(),
        enabled: true,
        name,
        version,
        mod_id: Some(mod_id),
        loader: loader.to_string(),
        authors,
        description,
        size: 0,
    })
}

/// 把 `key = value` 行拆成 (key, 值的原始右侧串)。等号前后空白被去除。
/// 找不到顶层等号返回 `None`。
fn split_kv(line: &str) -> Option<(&str, &str)> {
    let eq = line.find('=')?;
    let key = line[..eq].trim();
    let val = line[eq + 1..].trim();
    if key.is_empty() {
        return None;
    }
    Some((key, val))
}

/// 解析 TOML 标量值的右侧串,得到去引号后的字符串。
///
/// 支持:
///   - 多行字符串起始 `'''` / `"""`:持续读取后续行直到遇到对应结束定界符;
///   - 单行带引号 `"..."` / `'...'`:去掉首尾引号;
///   - 裸值:去掉行内注释后原样返回(容错)。
fn read_toml_value<'a, I: Iterator<Item = &'a str>>(
    first: &str,
    rest: &mut std::iter::Peekable<I>,
) -> String {
    // 多行字符串。
    for delim in ["'''", "\"\"\""] {
        if let Some(after) = first.strip_prefix(delim) {
            // 同一行内即闭合?
            if let Some(end) = after.find(delim) {
                return after[..end].to_string();
            }
            // 跨多行:逐行收集到结束定界符。
            let mut collected = String::from(after);
            for line in rest.by_ref() {
                if let Some(end) = line.find(delim) {
                    collected.push('\n');
                    collected.push_str(&line[..end]);
                    return collected.trim().to_string();
                }
                collected.push('\n');
                collected.push_str(line);
            }
            return collected.trim().to_string();
        }
    }

    // 单行:先剥掉可能的行内注释(仅当注释在引号外时;简单起见,带引号值不剥注释)。
    let trimmed = first.trim();
    if let Some(inner) = strip_quoted(trimmed) {
        return inner.to_string();
    }

    // 裸值:去掉 `#` 之后的注释。
    let no_comment = match trimmed.find('#') {
        Some(i) => trimmed[..i].trim(),
        None => trimmed,
    };
    no_comment.to_string()
}

/// 若 `s` 被一对相同引号包裹(`"..."` 或 `'...'`),返回去引号的内部串;否则 `None`。
fn strip_quoted(s: &str) -> Option<&str> {
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return Some(&s[1..s.len() - 1]);
        }
    }
    None
}

/// 把 authors 串拆成列表。Forge 习惯用逗号或 `and` 分隔(如 `"Alice, Bob and Carol"`)。
fn split_authors(raw: &str) -> Vec<String> {
    raw.split([',', ';'])
        .flat_map(|seg| seg.split(" and "))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    /// 在临时目录搭一个最小实例(只需要 mods/ 目录),测试结束自动清理。
    struct TempInst {
        root: PathBuf,
        inst: Instance,
    }

    impl TempInst {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("mc-core-mods-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let inst = Instance::new("1.20.1", root.clone());
            fs::create_dir_all(inst.mods_dir()).unwrap();
            Self { root, inst }
        }
    }

    impl Drop for TempInst {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// 构造一个内含单个文本条目的最小 jar(zip)写到指定路径。
    fn write_jar_with_entry(path: &Path, entry_name: &str, content: &str) {
        let file = fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zw.start_file(entry_name, opts).unwrap();
        zw.write_all(content.as_bytes()).unwrap();
        zw.finish().unwrap();
    }

    #[test]
    fn lists_and_parses_fabric_mod() {
        let t = TempInst::new("fabric");
        let jar = t.inst.mods_dir().join("sodium.jar");
        let fmj = r#"{
            "schemaVersion": 1,
            "id": "sodium",
            "version": "0.5.3",
            "name": "Sodium",
            "description": "A rendering engine.",
            "authors": ["JellySquid", {"name": "Contributor X"}]
        }"#;
        write_jar_with_entry(&jar, "fabric.mod.json", fmj);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        let m = &mods[0];
        assert_eq!(m.file_name, "sodium.jar");
        assert!(m.enabled);
        assert_eq!(m.name, "Sodium");
        assert_eq!(m.version.as_deref(), Some("0.5.3"));
        assert_eq!(m.mod_id.as_deref(), Some("sodium"));
        assert_eq!(m.loader, "fabric");
        assert_eq!(m.description.as_deref(), Some("A rendering engine."));
        assert_eq!(m.authors, vec!["JellySquid", "Contributor X"]);
        assert!(m.size > 0);
    }

    #[test]
    fn parses_quilt_mod() {
        let t = TempInst::new("quilt");
        let jar = t.inst.mods_dir().join("qmod.jar");
        let qmj = r#"{
            "schema_version": 1,
            "quilt_loader": {
                "id": "example_mod",
                "version": "1.2.3",
                "metadata": {
                    "name": "Example Quilt Mod",
                    "description": "Quilt test.",
                    "contributors": { "Alice": "Owner", "Bob": "Author" }
                }
            }
        }"#;
        write_jar_with_entry(&jar, "quilt.mod.json", qmj);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        let m = &mods[0];
        assert_eq!(m.name, "Example Quilt Mod");
        assert_eq!(m.version.as_deref(), Some("1.2.3"));
        assert_eq!(m.mod_id.as_deref(), Some("example_mod"));
        assert_eq!(m.loader, "quilt");
        assert_eq!(m.description.as_deref(), Some("Quilt test."));
        let mut authors = m.authors.clone();
        authors.sort();
        assert_eq!(authors, vec!["Alice", "Bob"]);
    }

    #[test]
    fn parses_forge_mods_toml() {
        let t = TempInst::new("forge");
        let jar = t.inst.mods_dir().join("forgemod.jar");
        let toml = r#"
modLoader = "javafml"
loaderVersion = "[47,)"
license = "MIT"
authors = "Top Author"

[[mods]]
modId = "examplemod"
version = "1.0.0"
displayName = "Example Forge Mod"
authors = "Alice, Bob and Carol"
description = '''
A multi-line
forge description.
'''
"#;
        write_jar_with_entry(&jar, "META-INF/mods.toml", toml);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        let m = &mods[0];
        assert_eq!(m.name, "Example Forge Mod");
        assert_eq!(m.version.as_deref(), Some("1.0.0"));
        assert_eq!(m.mod_id.as_deref(), Some("examplemod"));
        assert_eq!(m.loader, "forge");
        assert_eq!(m.authors, vec!["Alice", "Bob", "Carol"]);
        assert!(m.description.as_deref().unwrap().contains("multi-line"));
    }

    #[test]
    fn parses_neoforge_toml_and_prefers_it() {
        let t = TempInst::new("neoforge");
        let jar = t.inst.mods_dir().join("nfmod.jar");
        let toml = r#"
modLoader = "javafml"
[[mods]]
modId = "neomod"
version = "2.0.0"
displayName = "Neo Mod"
"#;
        write_jar_with_entry(&jar, "META-INF/neoforge.mods.toml", toml);

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].loader, "neoforge");
        assert_eq!(mods[0].mod_id.as_deref(), Some("neomod"));
        assert_eq!(mods[0].name, "Neo Mod");
    }

    #[test]
    fn unknown_jar_falls_back_to_filename() {
        let t = TempInst::new("unknown");
        let jar = t.inst.mods_dir().join("MysteryMod-1.0.jar");
        // 内含一个无关条目,既不是 fabric 也不是 forge。
        write_jar_with_entry(&jar, "pack.mcmeta", "{}");

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "MysteryMod-1.0");
        assert_eq!(mods[0].loader, "unknown");
        assert!(mods[0].enabled);
    }

    #[test]
    fn corrupt_jar_is_skipped_not_panicked() {
        let t = TempInst::new("corrupt");
        // 写一个非 zip 文件但以 .jar 结尾。
        fs::write(t.inst.mods_dir().join("broken.jar"), b"not a zip at all").unwrap();
        let mods = list_mods(&t.inst);
        // 不 panic,且退化为文件名条目。
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name, "broken");
        assert_eq!(mods[0].loader, "unknown");
    }

    #[test]
    fn disabled_state_detected_and_toggled() {
        let t = TempInst::new("toggle");
        let jar = t.inst.mods_dir().join("togglemod.jar");
        write_jar_with_entry(
            &jar,
            "fabric.mod.json",
            r#"{"id":"togglemod","name":"Toggle","version":"1.0"}"#,
        );

        // 初始启用。
        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert!(mods[0].enabled);
        assert_eq!(mods[0].file_name, "togglemod.jar");

        // 停用 → 文件应被重命名为 .disabled。
        set_mod_enabled(&t.inst, "togglemod.jar", false).unwrap();
        assert!(!t.inst.mods_dir().join("togglemod.jar").exists());
        assert!(t.inst.mods_dir().join("togglemod.jar.disabled").exists());

        let mods = list_mods(&t.inst);
        assert_eq!(mods.len(), 1);
        assert!(!mods[0].enabled);
        assert_eq!(mods[0].file_name, "togglemod.jar.disabled");
        // 即使停用,元数据仍应能从 jar 内读出。
        assert_eq!(mods[0].name, "Toggle");

        // 用带 .disabled 的文件名重新启用,且接受已是目标态时的幂等。
        set_mod_enabled(&t.inst, "togglemod.jar.disabled", true).unwrap();
        assert!(t.inst.mods_dir().join("togglemod.jar").exists());
        set_mod_enabled(&t.inst, "togglemod.jar", true).unwrap(); // 幂等 no-op
        assert!(t.inst.mods_dir().join("togglemod.jar").exists());
    }

    #[test]
    fn delete_removes_file() {
        let t = TempInst::new("delete");
        let jar = t.inst.mods_dir().join("doomed.jar");
        write_jar_with_entry(&jar, "fabric.mod.json", r#"{"id":"doomed"}"#);
        assert!(jar.exists());

        delete_mod(&t.inst, "doomed.jar").unwrap();
        assert!(!jar.exists());

        // 再删一次:文件已不存在,应幂等成功。
        delete_mod(&t.inst, "doomed.jar").unwrap();
    }

    #[test]
    fn rejects_path_traversal_file_name() {
        let t = TempInst::new("traversal");
        // 含 .. 或分隔符的文件名必须被拒绝,绝不逃出 mods/ 误删/改名其它文件。
        assert!(delete_mod(&t.inst, "../evil.jar").is_err());
        assert!(delete_mod(&t.inst, "sub/evil.jar").is_err());
        assert!(set_mod_enabled(&t.inst, "../evil.jar", false).is_err());
        assert!(set_mod_enabled(&t.inst, "a\\b.jar", true).is_err());
    }

    #[test]
    fn superseded_removes_same_mod_id_only() {
        let t = TempInst::new("supersede");
        let dir = t.inst.mods_dir();
        // 同一个 mod 的两个版本(相同 mod_id "sodium",不同文件名)。
        write_jar_with_entry(&dir.join("sodium-0.5.3.jar"), "fabric.mod.json", r#"{"id":"sodium","version":"0.5.3"}"#);
        write_jar_with_entry(&dir.join("sodium-0.5.8.jar"), "fabric.mod.json", r#"{"id":"sodium","version":"0.5.8"}"#);
        // 无关的另一个 mod,绝不能被动。
        write_jar_with_entry(&dir.join("fabric-api.jar"), "fabric.mod.json", r#"{"id":"fabric"}"#);
        // 同 mod 的旧版处于停用态,也应被清理(否则重新启用又冲突)。
        write_jar_with_entry(&dir.join("sodium-old.jar.disabled"), "fabric.mod.json", r#"{"id":"sodium","version":"0.4.0"}"#);

        let mut removed = remove_superseded(&t.inst, "sodium-0.5.8.jar").unwrap();
        removed.sort();

        assert!(dir.join("sodium-0.5.8.jar").exists(), "新版本应保留");
        assert!(dir.join("fabric-api.jar").exists(), "无关 mod 应保留");
        assert!(!dir.join("sodium-0.5.3.jar").exists(), "同 mod 旧版应清理");
        assert!(!dir.join("sodium-old.jar.disabled").exists(), "停用的同 mod 旧版也应清理");
        assert_eq!(
            removed,
            vec!["sodium-0.5.3.jar".to_string(), "sodium-old.jar.disabled".to_string()]
        );
    }

    #[test]
    fn superseded_noop_when_new_jar_unreadable() {
        // 新文件读不出 mod_id(损坏/无元数据)时,不得删除任何东西 —— 无法可靠判定归属。
        let t = TempInst::new("supersede-unknown");
        let dir = t.inst.mods_dir();
        write_jar_with_entry(&dir.join("sodium-0.5.3.jar"), "fabric.mod.json", r#"{"id":"sodium"}"#);
        fs::write(dir.join("mystery.jar"), b"not a zip").unwrap();

        let removed = remove_superseded(&t.inst, "mystery.jar").unwrap();
        assert!(removed.is_empty());
        assert!(dir.join("sodium-0.5.3.jar").exists(), "判定不了时旧文件必须原样保留");
    }

    #[test]
    fn missing_mods_dir_returns_empty() {
        let root = std::env::temp_dir()
            .join(format!("mc-core-mods-test-nomods-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let inst = Instance::new("1.20.1", root.clone());
        // 注意:不创建 mods 目录。
        assert!(list_mods(&inst).is_empty());
        let _ = fs::remove_dir_all(&root);
    }
}
