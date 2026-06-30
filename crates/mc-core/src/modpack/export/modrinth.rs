//! Modrinth `.mrpack` 导出目标。
//!
//! - **反查**:sha512(`provider.resolve_by_hashes(Sha512, …)`)。
//! - **远程引用门**:仅当解析出的下载 host 在 **mrpack 白名单**内
//!   ([`MRPACK_DOWNLOAD_HOSTS`](crate::modpack::formats::mrpack::MRPACK_DOWNLOAD_HOSTS))
//!   才允许写进索引;否则即便反查命中也回落 `overrides/`(Modrinth 规范只准从白名单 host 下载)。
//! - **索引**:`modrinth.index.json`(`formatVersion=1`),`files[]` 取自 resolved 集
//!   (path / hashes.sha512 / downloads / fileSize / env),`dependencies` 取自实例的
//!   mc 版本 + loader 图。复用 [`crate::modpack::formats::mrpack`] 的字段模型,**不**另起 schema。
//! - **打包**:zip + `overrides/`,排除 resolved 键。
//!
//! 这里同时给出 [`build_index`] 纯函数(从 [`ClassifiedSet`] + [`ExportInput`] 生成 [`MrpackIndex`]),
//! 可不经磁盘单测;`write_index` 只是它 + `serde_json` 序列化的薄封装。

use std::path::Path;

use mc_types::LoaderKind;

use crate::error::{CoreError, Result};
use crate::modpack::formats::mrpack::{
    EnvSupport, MrpackDependencies, MrpackEnv, MrpackFile, MrpackHashes, MrpackIndex,
    MRPACK_DOWNLOAD_HOSTS,
};
use crate::modplatform::{HashAlgo, ProviderId, ResolvedFile};

use super::walk::{matches_gate, MOD_DIR_PREFIXES};
use super::{ClassifiedSet, ExportInput, ExportTarget, Packaging};

/// `modrinth.index.json` 在 `.mrpack` 内的固定路径。
const MRPACK_INDEX_ENTRY: &str = "modrinth.index.json";

/// Modrinth `.mrpack` 导出目标。
#[derive(Debug, Clone, Default)]
pub struct ModrinthExportTarget;

impl ModrinthExportTarget {
    pub fn new() -> Self {
        Self
    }
}

impl ExportTarget for ModrinthExportTarget {
    fn id(&self) -> &'static str {
        "modrinth"
    }
    fn output_extension(&self) -> &'static str {
        "mrpack"
    }
    fn provider(&self) -> Option<ProviderId> {
        Some(ProviderId::Modrinth)
    }
    fn hash_algo(&self) -> Option<HashAlgo> {
        Some(HashAlgo::Sha512)
    }

    /// 门控:`mods/ coremods/ resourcepacks/ texturepacks/ shaderpacks/` × `{jar,litemod,zip}`。
    fn accepts(&self, relative: &Path) -> bool {
        let rel = relative.to_string_lossy();
        matches_gate(&rel, MOD_DIR_PREFIXES, &["jar", "litemod", "zip"])
    }

    /// 仅当下载 host 在 mrpack 白名单内才允许远程引用。
    fn allow_remote(&self, r: &ResolvedFile) -> bool {
        host_in_whitelist(&r.file.url)
    }

    fn write_index(&self, input: &ExportInput<'_>, set: &ClassifiedSet) -> Result<Vec<(String, Vec<u8>)>> {
        let index = build_index(input, set);
        let bytes = serde_json::to_vec_pretty(&index)
            .map_err(|e| CoreError::Parse { what: MRPACK_INDEX_ENTRY.into(), source: e })?;
        Ok(vec![(MRPACK_INDEX_ENTRY.to_string(), bytes)])
    }

    fn packaging(&self) -> Packaging {
        Packaging::ZipWithOverrides
    }
}

/// 判断一个 URL 的 host 是否在 mrpack 白名单内([`MRPACK_DOWNLOAD_HOSTS`]):等于某项或为其
/// 子域。解析与后缀匹配都委托给 [`crate::host`];导出侧把 host 归一为小写后再匹配(与重构前
/// `extract_host` 的行为一致),避免 `evilmodrinth.com` 蒙混。
pub fn host_in_whitelist(url: &str) -> bool {
    match crate::host::host_of(url) {
        Some(host) => {
            crate::host::host_matches_suffix(&host.to_ascii_lowercase(), MRPACK_DOWNLOAD_HOSTS)
        }
        None => false,
    }
}

/// 把 [`ClassifiedSet`] + [`ExportInput`] 纯函数式地构造成 [`MrpackIndex`](不触磁盘)。
///
/// - `files[]`:每个 resolved 文件一条,`path` = 相对路径,`hashes.sha512` = 解析文件哈希
///   (无则空串),`downloads` = `[url]`,`fileSize` = 解析大小,`env` = 客户端 + 服务端都 required
///   (导出侧无端别信息,保守标全需要;与导入侧 `client_supported` 缺省一致)。
/// - `dependencies`:`minecraft` = mc 版本;若有 loader,按家族填对应键。
pub fn build_index(input: &ExportInput<'_>, set: &ClassifiedSet) -> MrpackIndex {
    let files = set
        .resolved
        .iter()
        .map(|(rel, r)| MrpackFile {
            path: rel.to_string_lossy().replace('\\', "/"),
            hashes: MrpackHashes {
                sha512: r.file.sha512.clone().unwrap_or_default(),
                sha1: r.file.sha1.clone(),
            },
            env: Some(MrpackEnv {
                client: EnvSupport::Required,
                server: EnvSupport::Required,
            }),
            downloads: vec![r.file.url.clone()],
            file_size: r.file.size,
        })
        .collect();

    MrpackIndex {
        format_version: 1,
        game: "minecraft".to_string(),
        version_id: input.pack_version.clone().unwrap_or_default(),
        name: input.pack_name.clone(),
        summary: input.summary.clone(),
        dependencies: build_dependencies(&input.mc_version, input.loader.as_ref()),
        files,
    }
}

/// 由 mc 版本 + loader 图构造 `dependencies`。loader 家族映射到 mrpack 规范键。
fn build_dependencies(mc_version: &str, loader: Option<&(LoaderKind, String)>) -> MrpackDependencies {
    let mut deps = MrpackDependencies {
        minecraft: Some(mc_version.to_string()),
        ..Default::default()
    };
    if let Some((kind, ver)) = loader {
        match kind {
            LoaderKind::Fabric => deps.fabric_loader = Some(ver.clone()),
            LoaderKind::Quilt => deps.quilt_loader = Some(ver.clone()),
            LoaderKind::Forge => deps.forge = Some(ver.clone()),
            LoaderKind::NeoForge => deps.neoforge = Some(ver.clone()),
            // Vanilla / LiteLoader / OptiFine 在 mrpack 无对应依赖键:仅 minecraft。
            _ => {}
        }
    }
    deps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modplatform::{ProjectSideSupport, VersionFile};
    use std::path::PathBuf;

    fn resolved(rel: &str, url: &str, sha512: &str) -> (PathBuf, ResolvedFile) {
        (
            PathBuf::from(rel),
            ResolvedFile {
                provider: ProviderId::Modrinth,
                project_id: "PID".into(),
                version_id: "VID".into(),
                file: VersionFile {
                    url: url.into(),
                    filename: rel.rsplit('/').next().unwrap().into(),
                    sha1: Some("aa11".into()),
                    sha512: Some(sha512.into()),
                    size: Some(4096),
                    primary: true,
                    client_side: ProjectSideSupport::Unknown,
                    server_side: ProjectSideSupport::Unknown,
                },
                project_name: Some("Sodium".into()),
                project_slug: Some("sodium".into()),
                authors: vec!["jellysquid".into()],
            },
        )
    }

    #[test]
    fn host_whitelist_allows_canonical_and_subdomains_only() {
        assert!(host_in_whitelist("https://cdn.modrinth.com/data/x/a.jar"));
        assert!(host_in_whitelist("https://github.com/u/r/releases/a.jar"));
        assert!(host_in_whitelist("https://raw.githubusercontent.com/u/r/a.jar"));
        assert!(host_in_whitelist("https://media.gitlab.com/a.jar")); // 子域名
        assert!(host_in_whitelist("https://cdn.modrinth.com:443/a.jar")); // 带端口
        // 不在白名单。
        assert!(!host_in_whitelist("https://edge.forgecdn.net/files/a.jar"));
        assert!(!host_in_whitelist("https://example.com/a.jar"));
        // 防蒙混:evilmodrinth.com 不应命中。
        assert!(!host_in_whitelist("https://evilmodrinth.com/a.jar"));
        assert!(!host_in_whitelist("not a url"));
    }

    #[test]
    fn build_index_from_classified_set_is_pure() {
        let mut set = ClassifiedSet::default();
        set.resolved.push(resolved(
            "mods/sodium.jar",
            "https://cdn.modrinth.com/data/PID/sodium.jar",
            "longsha512",
        ));
        set.overrides.push(PathBuf::from("config/opts.toml"));

        let game_root = std::env::temp_dir();
        let mut input = ExportInput::new(&game_root, "My Pack", "1.20.1");
        input.pack_version = Some("1.2.3".into());
        input.summary = Some("a cool pack".into());
        input.loader = Some((LoaderKind::Fabric, "0.15.7".into()));

        let index = build_index(&input, &set);

        assert_eq!(index.format_version, 1);
        assert_eq!(index.game, "minecraft");
        assert_eq!(index.name, "My Pack");
        assert_eq!(index.version_id, "1.2.3");
        assert_eq!(index.summary.as_deref(), Some("a cool pack"));
        assert_eq!(index.dependencies.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(index.dependencies.fabric_loader.as_deref(), Some("0.15.7"));
        assert!(index.dependencies.forge.is_none());

        // files[] 只含 resolved(override 不进 files,走 overrides/)。
        assert_eq!(index.files.len(), 1);
        let f = &index.files[0];
        assert_eq!(f.path, "mods/sodium.jar");
        assert_eq!(f.hashes.sha512, "longsha512");
        assert_eq!(f.hashes.sha1.as_deref(), Some("aa11"));
        assert_eq!(f.downloads, vec!["https://cdn.modrinth.com/data/PID/sodium.jar"]);
        assert_eq!(f.file_size, Some(4096));
        let env = f.env.unwrap();
        assert_eq!(env.client, EnvSupport::Required);
        assert_eq!(env.server, EnvSupport::Required);
    }

    #[test]
    fn build_index_serializes_to_valid_mrpack_json() {
        // 生成的索引应能被同一 MrpackIndex 模型解析回来(往返契约)。
        let mut set = ClassifiedSet::default();
        set.resolved.push(resolved(
            "mods/lithium.jar",
            "https://cdn.modrinth.com/data/x/lithium.jar",
            "h512",
        ));
        let game_root = std::env::temp_dir();
        let input = ExportInput::new(&game_root, "RT", "1.21");
        let index = build_index(&input, &set);
        let json = serde_json::to_string(&index).unwrap();
        assert!(json.contains("\"formatVersion\":1"));
        let back: MrpackIndex = serde_json::from_str(&json).unwrap();
        assert_eq!(back.files[0].path, "mods/lithium.jar");
        assert_eq!(back.dependencies.minecraft.as_deref(), Some("1.21"));
    }

    #[test]
    fn neoforge_and_no_loader_dependency_mapping() {
        let game_root = std::env::temp_dir();
        let set = ClassifiedSet::default();

        let mut nf = ExportInput::new(&game_root, "NF", "1.20.1");
        nf.loader = Some((LoaderKind::NeoForge, "47.1.0".into()));
        let idx = build_index(&nf, &set);
        assert_eq!(idx.dependencies.neoforge.as_deref(), Some("47.1.0"));

        // 无 loader → 仅 minecraft 依赖。
        let vanilla = ExportInput::new(&game_root, "V", "1.20.1");
        let idxv = build_index(&vanilla, &set);
        assert_eq!(idxv.dependencies.minecraft.as_deref(), Some("1.20.1"));
        assert!(idxv.dependencies.fabric_loader.is_none());
        assert!(idxv.dependencies.neoforge.is_none());
    }
}
