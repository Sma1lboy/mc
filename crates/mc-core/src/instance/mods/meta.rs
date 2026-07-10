use super::*;

// ───────────────────────── 内部:文件名与兜底 ─────────────────────────

/// 去掉末尾的 `.disabled`(若有),得到"启用态"基名。
pub(crate) fn strip_disabled(name: &str) -> &str {
    name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name)
}

/// 由文件名构造兜底元数据:name = 去掉 `.jar`/`.jar.disabled` 的名字,loader = unknown。
pub(crate) fn fallback_info(file_name: &str) -> ModInfo {
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
pub(crate) fn read_mod_metadata(path: &Path) -> Option<ModInfo> {
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
pub(crate) fn read_entry<R: std::io::Read + std::io::Seek>(
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
pub(crate) fn parse_fabric(text: &str) -> Option<ModInfo> {
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
pub(crate) fn parse_quilt(text: &str) -> Option<ModInfo> {
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
pub(crate) fn author_to_string(v: &serde_json::Value) -> Option<String> {
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
pub(crate) fn contributors_to_authors(v: &serde_json::Value) -> Vec<String> {
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
pub(crate) fn parse_forge_toml(text: &str, loader: &str) -> Option<ModInfo> {
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
pub(crate) fn split_kv(line: &str) -> Option<(&str, &str)> {
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
pub(crate) fn read_toml_value<'a, I: Iterator<Item = &'a str>>(
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
pub(crate) fn strip_quoted(s: &str) -> Option<&str> {
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return Some(&s[1..s.len() - 1]);
        }
    }
    None
}

/// 把 authors 串拆成列表。Forge 习惯用逗号或 `and` 分隔(如 `"Alice, Bob and Carol"`)。
pub(crate) fn split_authors(raw: &str) -> Vec<String> {
    raw.split([',', ';'])
        .flat_map(|seg| seg.split(" and "))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}
