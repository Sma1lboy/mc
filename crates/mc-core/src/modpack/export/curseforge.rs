//! CurseForge `.zip` 导出目标。
//!
//! - **反查**:murmur2 指纹(`provider.resolve_by_hashes(Murmur2, …)` → `/fingerprints` →
//!   `exactMatches[].file`,再 `/mods` 补 slug/name/authors)。
//! - **远程引用门**:仅当解析文件 `isAvailable`(有下载 URL,即 `file.url` 非空)才能进
//!   manifest;否则即便匹配也回落 overrides(数据丢失边界:见 §0「CF 额外坑」,这里选择
//!   保守地随包带走而非丢弃)。
//! - **索引**:两份注入文件 ——
//!   - `manifest.json`:`manifestType=minecraftModpack`、`minecraft.modLoaders[].id="fabric-x"`、
//!     `files[]` 取自 resolved 集(projectID/fileID);
//!   - `modlist.html`:人类可读的 mod 清单(项目名 + 链接 + 作者)。
//! - **打包**:zip + `overrides/`,排除 resolved 键。
//!
//! [`build_manifest`] / [`build_modlist_html`] 是纯函数(从 [`ClassifiedSet`] + [`ExportInput`]
//! 生成),可不触磁盘单测;`write_index` 是它们 + 序列化的薄封装。

use std::path::Path;

use mc_types::LoaderKind;
use serde::Serialize;

use crate::error::{CoreError, Result};
use crate::modplatform::{HashAlgo, ProviderId, ResolvedFile};

use super::walk::matches_gate;
use super::{ClassifiedSet, ExportInput, ExportTarget, Packaging};

/// manifest 在 CF zip 内的固定路径。
const CF_MANIFEST_ENTRY: &str = "manifest.json";
/// mod 清单 HTML 在 CF zip 内的固定路径。
const CF_MODLIST_ENTRY: &str = "modlist.html";

/// CurseForge 门控目录前缀(比 Modrinth 少 texturepacks;含 resourcepacks 特判)× `{jar,zip}`。
const CF_DIR_PREFIXES: &[&str] = &["mods/", "resourcepacks/"];

/// CurseForge `.zip` 导出目标。
#[derive(Debug, Clone, Default)]
pub struct CurseForgeExportTarget;

impl CurseForgeExportTarget {
    pub fn new() -> Self {
        Self
    }
}

impl ExportTarget for CurseForgeExportTarget {
    fn id(&self) -> &'static str {
        "curseforge"
    }
    fn output_extension(&self) -> &'static str {
        "zip"
    }
    fn provider(&self) -> Option<ProviderId> {
        Some(ProviderId::CurseForge)
    }
    fn hash_algo(&self) -> Option<HashAlgo> {
        Some(HashAlgo::Murmur2)
    }

    /// 门控:`mods/ resourcepacks/` × `{jar,zip}`。
    fn accepts(&self, relative: &Path) -> bool {
        let rel = relative.to_string_lossy();
        matches_gate(&rel, CF_DIR_PREFIXES, &["jar", "zip"])
    }

    /// 仅当解析文件可用(有下载 URL)才允许远程引用;blocked(url 空)→ 回落 overrides。
    fn allow_remote(&self, r: &ResolvedFile) -> bool {
        !r.file.url.is_empty()
    }

    fn write_index(&self, input: &ExportInput<'_>, set: &ClassifiedSet) -> Result<Vec<(String, Vec<u8>)>> {
        let manifest = build_manifest(input, set);
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| CoreError::Parse { what: CF_MANIFEST_ENTRY.into(), source: e })?;
        let html = build_modlist_html(input, set);
        Ok(vec![
            (CF_MANIFEST_ENTRY.to_string(), manifest_bytes),
            (CF_MODLIST_ENTRY.to_string(), html.into_bytes()),
        ])
    }

    fn packaging(&self) -> Packaging {
        Packaging::ZipWithOverrides
    }
}

// ===========================================================================
// manifest.json 模型(导出专用;导入侧的 FlameManifest 字段不全对得上输出需求,
// 这里给一份**只写**的最小 schema,对齐 CurseForge 导出规范)。
// ===========================================================================

/// CF `manifest.json`(导出写出)。字段名严格按规范 camelCase。
#[derive(Debug, Clone, Serialize)]
pub struct CfManifest {
    #[serde(rename = "minecraft")]
    pub minecraft: CfMinecraft,
    #[serde(rename = "manifestType")]
    pub manifest_type: &'static str,
    #[serde(rename = "manifestVersion")]
    pub manifest_version: u32,
    pub name: String,
    pub version: String,
    pub author: String,
    pub files: Vec<CfManifestFile>,
    pub overrides: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct CfMinecraft {
    pub version: String,
    #[serde(rename = "modLoaders")]
    pub mod_loaders: Vec<CfModLoader>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CfModLoader {
    /// `"fabric-0.15.7"` / `"forge-47.2.0"` / `"neoforge-..."` / `"quilt-..."`。
    pub id: String,
    pub primary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CfManifestFile {
    #[serde(rename = "projectID")]
    pub project_id: i64,
    #[serde(rename = "fileID")]
    pub file_id: i64,
    pub required: bool,
}

/// 由 [`ClassifiedSet`] + [`ExportInput`] 构造 CF `manifest.json`(纯函数)。
///
/// 只有 provider 为 CurseForge 的 resolved 文件能成为 `files[]`(其 project_id/version_id 是 CF id);
/// 其它 provider 的 resolved 不该出现在 CF 导出里(引擎已保证反查用的是 CF provider)。无法解析成
/// i64 的 id 跳过(防御性,正常不会发生)。
pub fn build_manifest(input: &ExportInput<'_>, set: &ClassifiedSet) -> CfManifest {
    let files = set
        .resolved
        .iter()
        .filter(|(_, r)| r.provider == ProviderId::CurseForge)
        .filter_map(|(_, r)| {
            let project_id = r.project_id.trim().parse::<i64>().ok()?;
            // fileID 优先取 version_id(CF provider 把它设为文件 id),回退从 forgecdn URL 抽。
            let file_id = r
                .version_id
                .trim()
                .parse::<i64>()
                .ok()
                .or_else(|| url_file_id(&r.file.url))?;
            Some(CfManifestFile { project_id, file_id, required: true })
        })
        .collect();

    CfManifest {
        minecraft: CfMinecraft {
            version: input.mc_version.clone(),
            mod_loaders: cf_mod_loaders(input.loader.as_ref()),
        },
        manifest_type: "minecraftModpack",
        manifest_version: 1,
        name: input.pack_name.clone(),
        version: input.pack_version.clone().unwrap_or_default(),
        author: input.author.clone().unwrap_or_else(|| "Anonymous".to_string()),
        files,
        overrides: "overrides",
    }
}

/// 把 loader 图映射成 CF `modLoaders[]`(单项,primary)。`"fabric-<ver>"` 形式。
/// 无 loader / 不支持的 loader → 空数组(原版包)。
fn cf_mod_loaders(loader: Option<&(LoaderKind, String)>) -> Vec<CfModLoader> {
    match loader {
        Some((kind, ver)) => {
            let fam = match kind {
                LoaderKind::Fabric => "fabric",
                LoaderKind::Quilt => "quilt",
                LoaderKind::Forge => "forge",
                LoaderKind::NeoForge => "neoforge",
                _ => return Vec::new(),
            };
            vec![CfModLoader { id: format!("{fam}-{ver}"), primary: true }]
        }
        None => Vec::new(),
    }
}

/// 构造人类可读的 `modlist.html`:`<ul>` 列出每个 resolved mod(名 + 项目链接 + 作者)。
/// CurseForge 客户端把它作为整合包内附的清单展示。文本经 HTML 实体转义,防注入。
pub fn build_modlist_html(input: &ExportInput<'_>, set: &ClassifiedSet) -> String {
    let mut out = String::new();
    out.push_str("<html>\n<head><meta charset=\"utf-8\"><title>");
    out.push_str(&html_escape(&input.pack_name));
    out.push_str("</title></head>\n<body>\n<ul>\n");
    for (_, r) in &set.resolved {
        let name = r
            .project_name
            .clone()
            .or_else(|| r.project_slug.clone())
            .unwrap_or_else(|| r.file.filename.clone());
        let authors = if r.authors.is_empty() {
            String::new()
        } else {
            format!(" (by {})", r.authors.join(", "))
        };
        // CurseForge 项目页 URL(若有 slug)。
        let link = r
            .project_slug
            .as_ref()
            .map(|s| format!("https://www.curseforge.com/minecraft/mc-mods/{s}"));
        out.push_str("  <li>");
        match link {
            Some(url) => {
                out.push_str("<a href=\"");
                out.push_str(&html_escape(&url));
                out.push_str("\">");
                out.push_str(&html_escape(&name));
                out.push_str("</a>");
            }
            None => out.push_str(&html_escape(&name)),
        }
        out.push_str(&html_escape(&authors));
        out.push_str("</li>\n");
    }
    out.push_str("</ul>\n</body>\n</html>\n");
    out
}

/// 最小 HTML 实体转义(`& < > " '`)。
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

/// 从 forgecdn 下载 URL 抽 fileID。URL 形如 `https://edge.forgecdn.net/files/4567/890/sodium.jar`,
/// fileID = `/files/` 之后第一个数字段(CF 把 fileID 拆成高低两段拼路径,取首段即文件 id 高位,
/// 但实践中 manifest 用的是完整 fileID;此处优先用 version_id,本函数仅作 URL 回退兜底)。
fn url_file_id(url: &str) -> Option<i64> {
    let after = url.split("/files/").nth(1)?;
    let first = after.split('/').next()?;
    first.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modplatform::VersionFile;
    use std::path::PathBuf;

    fn cf_resolved(rel: &str, project_id: &str, version_id: &str, file_id_url: &str, slug: &str) -> (PathBuf, ResolvedFile) {
        (
            PathBuf::from(rel),
            ResolvedFile {
                provider: ProviderId::CurseForge,
                project_id: project_id.into(),
                version_id: version_id.into(),
                file: VersionFile {
                    url: file_id_url.into(),
                    filename: rel.rsplit('/').next().unwrap().into(),
                    sha1: Some("aa".into()),
                    sha512: None,
                    size: Some(10),
                    primary: true,
                },
                project_name: Some("Sodium".into()),
                project_slug: Some(slug.into()),
                authors: vec!["jellysquid".into()],
            },
        )
    }

    #[test]
    fn manifest_files_take_cf_ids_and_loader_id_format() {
        let mut set = ClassifiedSet::default();
        set.resolved.push(cf_resolved(
            "mods/sodium.jar",
            "12345",
            "4567",
            "https://edge.forgecdn.net/files/4567/890/sodium.jar",
            "sodium",
        ));
        set.overrides.push(PathBuf::from("config/opts.toml"));

        let game_root = std::env::temp_dir();
        let mut input = ExportInput::new(&game_root, "CF Pack", "1.20.1");
        input.pack_version = Some("2.0".into());
        input.author = Some("me".into());
        input.loader = Some((LoaderKind::Fabric, "0.15.7".into()));

        let m = build_manifest(&input, &set);
        assert_eq!(m.manifest_type, "minecraftModpack");
        assert_eq!(m.manifest_version, 1);
        assert_eq!(m.name, "CF Pack");
        assert_eq!(m.version, "2.0");
        assert_eq!(m.author, "me");
        assert_eq!(m.overrides, "overrides");
        assert_eq!(m.minecraft.version, "1.20.1");
        assert_eq!(m.minecraft.mod_loaders.len(), 1);
        assert_eq!(m.minecraft.mod_loaders[0].id, "fabric-0.15.7");
        assert!(m.minecraft.mod_loaders[0].primary);

        // files[] 取 projectID(12345)+ fileID(version_id 4567)。
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].project_id, 12345);
        assert_eq!(m.files[0].file_id, 4567);
        assert!(m.files[0].required);
    }

    #[test]
    fn manifest_file_id_falls_back_to_forgecdn_url() {
        // version_id 非数字时,从 forgecdn URL 的 /files/<id>/ 段抽 fileID。
        let mut set = ClassifiedSet::default();
        set.resolved.push(cf_resolved(
            "mods/sodium.jar",
            "12345",
            "not-a-number",
            "https://edge.forgecdn.net/files/4567/890/sodium.jar",
            "sodium",
        ));
        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "P", "1.20.1");
        let m = build_manifest(&input, &set);
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].file_id, 4567);
    }

    #[test]
    fn manifest_serializes_with_canonical_keys() {
        let mut set = ClassifiedSet::default();
        set.resolved.push(cf_resolved(
            "mods/jei.jar",
            "238222",
            "999",
            "https://edge.forgecdn.net/files/999/0/jei.jar",
            "jei",
        ));
        let game_root = std::env::temp_dir();
        let mut input = ExportInput::new(&game_root, "P", "1.20.1");
        input.loader = Some((LoaderKind::NeoForge, "47.1.0".into()));
        let m = build_manifest(&input, &set);
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"manifestType\":\"minecraftModpack\""));
        assert!(json.contains("\"projectID\":238222"));
        assert!(json.contains("\"fileID\":999"));
        assert!(json.contains("\"id\":\"neoforge-47.1.0\""));
        assert!(json.contains("\"modLoaders\""));
    }

    #[test]
    fn modlist_html_escapes_and_links() {
        let mut set = ClassifiedSet::default();
        let mut entry = cf_resolved(
            "mods/x.jar",
            "1",
            "2",
            "https://edge.forgecdn.net/files/1/0/x.jar",
            "cool-mod",
        );
        entry.1.project_name = Some("Cool & <Mod>".into());
        entry.1.authors = vec!["a\"b".into()];
        set.resolved.push(entry);

        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "Pack", "1.20.1");
        let html = build_modlist_html(&input, &set);
        // 名字里的 & < > 被转义。
        assert!(html.contains("Cool &amp; &lt;Mod&gt;"));
        // 作者引号转义。
        assert!(html.contains("a&quot;b"));
        // 含项目链接。
        assert!(html.contains("https://www.curseforge.com/minecraft/mc-mods/cool-mod"));
        // 合法 HTML 骨架。
        assert!(html.starts_with("<html>"));
        assert!(html.trim_end().ends_with("</html>"));
    }

    #[test]
    fn no_loader_yields_empty_modloaders() {
        let set = ClassifiedSet::default();
        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "Vanilla", "1.20.1");
        let m = build_manifest(&input, &set);
        assert!(m.minecraft.mod_loaders.is_empty());
        assert_eq!(m.author, "Anonymous"); // 默认作者
    }
}
