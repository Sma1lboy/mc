//! Mod 清单导出目标:把实例里的 mod 列成一份**单文本文件**(HTML / Markdown / 纯文本 /
//! JSON / CSV),证明「导出」不止打包 —— 它是 `Packaging::SingleTextFile` + `provider=None`
//! 的 [`ExportTarget`],resolve 阶段被引擎自动跳过(`docs/modules/modpack-export.md` §2/§5)。
//!
//! 因无 provider 反查,modlist 只能用**本地可得**的信息:文件名(总有)+ 从文件名启发式
//! 解析出的 mod 名 / 版本。`OptionalData{FileName}` 默认开;`Authors`/`Url` 无来源时留空列。
//! 各格式各自转义(HTML 实体、Markdown 标点、CSV 引号),互不串味。
//!
//! 引擎对 `provider()==None` 的目标把所有门控命中(本目标 `accepts` 收 `mods/*.jar|.disabled`)
//! 归入 `set.overrides`;modlist 的 `write_index` 从 `overrides` 读出文件清单生成文本,
//! 然后引擎按 `SingleTextFile` 写出这唯一字节(不打 zip、无 overrides 目录)。

use std::path::Path;

use crate::error::Result;
use crate::modplatform::{HashAlgo, ProviderId};

use super::{ClassifiedSet, ExportInput, ExportTarget, Packaging};

/// modlist 输出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModListFormat {
    Html,
    Markdown,
    PlainText,
    Json,
    Csv,
}

impl ModListFormat {
    /// 该格式的输出文件扩展名。
    pub fn extension(self) -> &'static str {
        match self {
            ModListFormat::Html => "html",
            ModListFormat::Markdown => "md",
            ModListFormat::PlainText => "txt",
            ModListFormat::Json => "json",
            ModListFormat::Csv => "csv",
        }
    }
}

/// 选列开关(`OptionalData`)。`Name` 总输出(列表的主键);其余可选。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModListColumns {
    /// 输出从文件名解析出的版本号列(默认 true)。
    pub version: bool,
    /// 输出文件名列(默认 true)。
    pub file_name: bool,
}

impl Default for ModListColumns {
    fn default() -> Self {
        Self { version: true, file_name: true }
    }
}

/// Mod 清单导出目标。
#[derive(Debug, Clone)]
pub struct ModListExportTarget {
    pub format: ModListFormat,
    pub columns: ModListColumns,
}

impl ModListExportTarget {
    /// 用指定格式 + 默认列构造。
    pub fn new(format: ModListFormat) -> Self {
        Self { format, columns: ModListColumns::default() }
    }
}

impl ExportTarget for ModListExportTarget {
    fn id(&self) -> &'static str {
        "modlist"
    }
    fn output_extension(&self) -> &'static str {
        self.format.extension()
    }
    fn provider(&self) -> Option<ProviderId> {
        None
    }
    fn hash_algo(&self) -> Option<HashAlgo> {
        None
    }

    /// 收 `mods/` 下的 `.jar` 与 `.disabled`(禁用的 mod 也列上,标注禁用)。
    fn accepts(&self, relative: &Path) -> bool {
        let rel = relative.to_string_lossy().replace('\\', "/");
        rel.starts_with("mods/") && (rel.ends_with(".jar") || rel.ends_with(".disabled"))
    }

    fn write_index(&self, input: &ExportInput<'_>, set: &ClassifiedSet) -> Result<Vec<(String, Vec<u8>)>> {
        // provider=None → 所有门控命中在 overrides 里;据此生成条目。
        let entries: Vec<ModEntry> = set
            .overrides
            .iter()
            .filter_map(|p| {
                let rel = p.to_string_lossy().replace('\\', "/");
                if self.accepts(Path::new(&rel)) {
                    Some(ModEntry::from_filename(rel.rsplit('/').next().unwrap_or(&rel)))
                } else {
                    None
                }
            })
            .collect();

        let text = self.render(input, &entries);
        Ok(vec![(format!("modlist.{}", self.format.extension()), text.into_bytes())])
    }

    fn packaging(&self) -> Packaging {
        Packaging::SingleTextFile
    }
}

/// 一条 mod 清单项(由本地文件名启发式解析而来)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModEntry {
    /// 展示名(去掉版本与扩展名后的文件名 stem)。
    pub name: String,
    /// 从文件名末尾切出的版本(如 `sodium-fabric-0.5.3.jar` → `0.5.3`);无则 None。
    pub version: Option<String>,
    /// 原始文件名(含扩展名)。
    pub file_name: String,
    /// 是否被禁用(`.disabled` 结尾)。
    pub disabled: bool,
}

impl ModEntry {
    /// 从 jar/disabled 文件名启发式解析名与版本。
    ///
    /// 规则:去掉 `.jar` / `.disabled`(可叠加,如 `a.jar.disabled`)得到 stem;若 stem 含 `-` 且
    /// 末段以数字开头(version-ish),把末段当版本、其前当名;否则整体当名、版本 None。
    pub fn from_filename(file_name: &str) -> Self {
        let mut stem = file_name;
        let mut disabled = false;
        if let Some(s) = stem.strip_suffix(".disabled") {
            stem = s;
            disabled = true;
        }
        if let Some(s) = stem.strip_suffix(".jar") {
            stem = s;
        }
        // 末段以数字开头视为版本。
        let (name, version) = match stem.rsplit_once('-') {
            Some((head, tail)) if tail.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) => {
                (head.to_string(), Some(tail.to_string()))
            }
            _ => (stem.to_string(), None),
        };
        ModEntry {
            name: if name.is_empty() { file_name.to_string() } else { name },
            version,
            file_name: file_name.to_string(),
            disabled,
        }
    }
}

impl ModListExportTarget {
    /// 把条目集渲染成目标格式文本(按格式分派,各自转义)。条目按名排序保证确定输出。
    fn render(&self, input: &ExportInput<'_>, entries: &[ModEntry]) -> String {
        let mut entries = entries.to_vec();
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.file_name.cmp(&b.file_name)));
        match self.format {
            ModListFormat::Html => self.render_html(input, &entries),
            ModListFormat::Markdown => self.render_markdown(input, &entries),
            ModListFormat::PlainText => self.render_plain(&entries),
            ModListFormat::Json => self.render_json(input, &entries),
            ModListFormat::Csv => self.render_csv(&entries),
        }
    }

    fn render_html(&self, input: &ExportInput<'_>, entries: &[ModEntry]) -> String {
        let mut out = String::new();
        out.push_str("<html>\n<head><meta charset=\"utf-8\"><title>");
        out.push_str(&html_escape(&input.pack_name));
        out.push_str("</title></head>\n<body>\n<ul>\n");
        for e in entries {
            out.push_str("  <li>");
            out.push_str(&html_escape(&e.name));
            if self.columns.version {
                if let Some(v) = &e.version {
                    out.push(' ');
                    out.push_str(&html_escape(v));
                }
            }
            if e.disabled {
                out.push_str(" (disabled)");
            }
            if self.columns.file_name {
                out.push_str(" <code>");
                out.push_str(&html_escape(&e.file_name));
                out.push_str("</code>");
            }
            out.push_str("</li>\n");
        }
        out.push_str("</ul>\n</body>\n</html>\n");
        out
    }

    fn render_markdown(&self, input: &ExportInput<'_>, entries: &[ModEntry]) -> String {
        let mut out = String::new();
        out.push_str("# ");
        out.push_str(&md_escape(&input.pack_name));
        out.push_str("\n\n");
        for e in entries {
            out.push_str("- ");
            out.push_str(&md_escape(&e.name));
            if self.columns.version {
                if let Some(v) = &e.version {
                    out.push(' ');
                    out.push_str(&md_escape(v));
                }
            }
            if e.disabled {
                out.push_str(" _(disabled)_");
            }
            if self.columns.file_name {
                out.push_str(" `");
                out.push_str(&e.file_name.replace('`', "'"));
                out.push('`');
            }
            out.push('\n');
        }
        out
    }

    fn render_plain(&self, entries: &[ModEntry]) -> String {
        let mut out = String::new();
        for e in entries {
            out.push_str(&e.name);
            if self.columns.version {
                if let Some(v) = &e.version {
                    out.push(' ');
                    out.push_str(v);
                }
            }
            if e.disabled {
                out.push_str(" (disabled)");
            }
            if self.columns.file_name {
                out.push_str(" [");
                out.push_str(&e.file_name);
                out.push(']');
            }
            out.push('\n');
        }
        out
    }

    fn render_json(&self, input: &ExportInput<'_>, entries: &[ModEntry]) -> String {
        // 手写最小 JSON(避免给 ModEntry 派生 Serialize 引入耦合;字段固定)。
        let items: Vec<String> = entries
            .iter()
            .map(|e| {
                let mut fields = vec![format!("\"name\":{}", json_str(&e.name))];
                if self.columns.version {
                    let v = e.version.as_deref().unwrap_or("");
                    fields.push(format!("\"version\":{}", json_str(v)));
                }
                if self.columns.file_name {
                    fields.push(format!("\"fileName\":{}", json_str(&e.file_name)));
                }
                fields.push(format!("\"disabled\":{}", e.disabled));
                format!("{{{}}}", fields.join(","))
            })
            .collect();
        format!(
            "{{\"name\":{},\"mcVersion\":{},\"mods\":[{}]}}",
            json_str(&input.pack_name),
            json_str(&input.mc_version),
            items.join(",")
        )
    }

    fn render_csv(&self, entries: &[ModEntry]) -> String {
        let mut out = String::new();
        // 表头。
        let mut header = vec!["name"];
        if self.columns.version {
            header.push("version");
        }
        if self.columns.file_name {
            header.push("fileName");
        }
        header.push("disabled");
        out.push_str(&header.join(","));
        out.push('\n');
        for e in entries {
            let mut cols = vec![csv_field(&e.name)];
            if self.columns.version {
                cols.push(csv_field(e.version.as_deref().unwrap_or("")));
            }
            if self.columns.file_name {
                cols.push(csv_field(&e.file_name));
            }
            cols.push(if e.disabled { "true".to_string() } else { "false".to_string() });
            out.push_str(&cols.join(","));
            out.push('\n');
        }
        out
    }
}

/// 最小 HTML 实体转义。
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Markdown 行内转义:对会破坏列表项的标点反斜杠转义。
fn md_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '*' | '_' | '`' | '[' | ']' | '(' | ')' | '#' | '\\' | '|' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// JSON 字符串字面量(含引号),手工转义控制字符与引号 / 反斜杠。
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// CSV 字段:含逗号 / 引号 / 换行时用双引号包裹并把内部引号翻倍。
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn set_with(files: &[&str]) -> ClassifiedSet {
        let mut set = ClassifiedSet::default();
        set.overrides = files.iter().map(PathBuf::from).collect();
        set
    }

    #[test]
    fn parses_name_and_version_from_filename() {
        let e = ModEntry::from_filename("sodium-fabric-0.5.3.jar");
        assert_eq!(e.name, "sodium-fabric");
        assert_eq!(e.version.as_deref(), Some("0.5.3"));
        assert_eq!(e.file_name, "sodium-fabric-0.5.3.jar");
        assert!(!e.disabled);

        // 无版本式末段。
        let e2 = ModEntry::from_filename("JustEnoughItems.jar");
        assert_eq!(e2.name, "JustEnoughItems");
        assert!(e2.version.is_none());

        // .disabled。
        let e3 = ModEntry::from_filename("oldmod-1.2.jar.disabled");
        assert!(e3.disabled);
        assert_eq!(e3.name, "oldmod");
        assert_eq!(e3.version.as_deref(), Some("1.2"));
    }

    #[test]
    fn target_is_provider_none_single_text() {
        let t = ModListExportTarget::new(ModListFormat::Html);
        assert!(t.provider().is_none());
        assert!(t.hash_algo().is_none());
        assert_eq!(t.packaging(), Packaging::SingleTextFile);
        assert_eq!(t.output_extension(), "html");
        // accepts 收 mods/*.jar|.disabled,拒其它。
        assert!(t.accepts(Path::new("mods/a.jar")));
        assert!(t.accepts(Path::new("mods/b.jar.disabled")));
        assert!(!t.accepts(Path::new("config/a.cfg")));
        assert!(!t.accepts(Path::new("resourcepacks/p.zip")));
    }

    #[test]
    fn html_format_escapes_and_lists_sorted() {
        let t = ModListExportTarget::new(ModListFormat::Html);
        let set = set_with(&["mods/Zmod-1.0.jar", "mods/Amod-2.0.jar", "config/x.cfg"]);
        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "My <Pack>", "1.20.1");
        let (name, bytes) = &t.write_index(&input, &set).unwrap()[0];
        assert_eq!(name, "modlist.html");
        let html = String::from_utf8(bytes.clone()).unwrap();
        // 标题转义。
        assert!(html.contains("My &lt;Pack&gt;"));
        // 按名排序:Amod 在 Zmod 之前。config/x.cfg 不是门控命中,不列。
        let a = html.find("Amod").unwrap();
        let z = html.find("Zmod").unwrap();
        assert!(a < z, "应按名排序");
        assert!(!html.contains("x.cfg"));
        // 版本与文件名列。
        assert!(html.contains("2.0"));
        assert!(html.contains("<code>Amod-2.0.jar</code>"));
    }

    #[test]
    fn csv_quotes_fields_with_commas() {
        let t = ModListExportTarget::new(ModListFormat::Csv);
        // 文件名里含逗号 → 该字段应被引号包裹。
        let set = set_with(&["mods/we,ird-1.0.jar"]);
        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "P", "1.20.1");
        let (_, bytes) = &t.write_index(&input, &set).unwrap()[0];
        let csv = String::from_utf8(bytes.clone()).unwrap();
        assert!(csv.starts_with("name,version,fileName,disabled\n"));
        assert!(csv.contains("\"we,ird-1.0.jar\""), "含逗号字段应加引号");
    }

    #[test]
    fn json_is_valid_and_has_mods_array() {
        let t = ModListExportTarget::new(ModListFormat::Json);
        let set = set_with(&["mods/sodium-0.5.3.jar"]);
        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "Pa\"ck", "1.20.1");
        let (_, bytes) = &t.write_index(&input, &set).unwrap()[0];
        let txt = String::from_utf8(bytes.clone()).unwrap();
        // 用 serde_json 反序列化校验是合法 JSON 且字段正确(包括引号转义)。
        let v: serde_json::Value = serde_json::from_str(&txt).unwrap();
        assert_eq!(v["name"], "Pa\"ck");
        assert_eq!(v["mcVersion"], "1.20.1");
        assert_eq!(v["mods"][0]["name"], "sodium");
        assert_eq!(v["mods"][0]["version"], "0.5.3");
        assert_eq!(v["mods"][0]["fileName"], "sodium-0.5.3.jar");
        assert_eq!(v["mods"][0]["disabled"], false);
    }

    #[test]
    fn markdown_escapes_special_chars() {
        let t = ModListExportTarget::new(ModListFormat::Markdown);
        let set = set_with(&["mods/cool_mod-1.0.jar"]);
        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "P", "1.20.1");
        let (name, bytes) = &t.write_index(&input, &set).unwrap()[0];
        assert_eq!(name, "modlist.md");
        let md = String::from_utf8(bytes.clone()).unwrap();
        assert!(md.starts_with("# P\n"));
        // 下划线被转义。
        assert!(md.contains("cool\\_mod"));
    }
}
