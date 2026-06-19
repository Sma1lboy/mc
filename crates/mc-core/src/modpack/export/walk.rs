//! 共享导出遍历:递归 `game_root` 收集文件 + 套硬忽略规则 + 算哈希(sha1/sha512/murmur2)
//! + **path-safety**(全部相对 `game_root`,任何逃逸即跳过)。
//!
//! 这是导出五阶段管线的第 1、2 步底座,**与目标格式无关**:目标只通过
//! [`super::ExportTarget::accepts`] 决定一个候选文件是否需要哈希反查(进 resolve 阶段),
//! 不进门控的文件直接当 override 打包,**不**哈希。本模块只负责把磁盘上的文件枚举成一组
//! 带相对路径(`/` 分隔)的 [`WalkedFile`],并按需对其计算哈希。
//!
//! 设计要点:
//! - **硬忽略**([`is_hard_ignored`]):`logs/ crash-reports/ .cache/ .fabric/ .quilt/` 目录、
//!   `.DS_Store / thumbs.db` 文件、`*.pw.toml`(packwiz 元数据)永不导出。纯函数,可单测。
//! - **用户忽略**:调用方传入一组相对路径前缀(来自 `<instance>/.packignore`),命中即跳。
//! - **path-safety**:虽然遍历的是本地真实目录(不存在 zip-slip),仍统一经
//!   [`crate::fs::safe_join`] 把每个相对路径收口回 `game_root`,符号链接逃逸即丢弃 —— 与导入侧
//!   解压安全对称(见 `docs/modules/modpack-export.md` §5)。
//! - **哈希懒算**:遍历只记录相对路径与绝对路径;哈希在 [`WalkedFile::hash`] 里按目标声明的
//!   单一算法计算,避免对不进 resolve 的文件做无谓哈希。

use std::path::{Path, PathBuf};

use crate::download::checksum::{sha1_file, sha512_file};
use crate::download::murmur2::cf_fingerprint_file;
use crate::error::Result;
use crate::modplatform::HashAlgo;

/// 永不导出的硬忽略**目录**(相对 game_root 的首段;命中其下全部内容)。
///
/// 见 `docs/modules/modpack-export.md` §1:这些是运行时产物 / 工具缓存,导出无意义且会泄漏
/// 本机痕迹。各资源的 `.index` 元数据目录(packwiz sidecar)也一并忽略。
const IGNORED_DIRS: &[&str] = &[
    "logs",
    "crash-reports",
    ".cache",
    ".fabric",
    ".quilt",
    ".mixin.out",
    ".index",
];

/// 永不导出的硬忽略**文件 basename**(大小写不敏感)。
const IGNORED_FILES: &[&str] = &[".DS_Store", "thumbs.db"];

/// 永不导出的硬忽略**文件后缀**(packwiz 的 per-mod sidecar TOML)。
const IGNORED_SUFFIXES: &[&str] = &[".pw.toml"];

/// 遍历产出的一个候选文件:相对 `game_root` 的路径(`/` 分隔,稳定排序)+ 绝对路径。
#[derive(Debug, Clone)]
pub struct WalkedFile {
    /// 相对 `game_root` 的路径,`/` 分隔(供 zip 条目名 / 索引 path 用)。
    pub rel: String,
    /// 绝对路径(供哈希 / 读取打包)。
    pub abs: PathBuf,
    /// 字节大小(遍历时随手读元数据,免二次 stat)。
    pub size: u64,
}

impl WalkedFile {
    /// 按单一算法计算该文件哈希(十六进制小写)。`Murmur2` 返回十进制无符号串
    /// (CurseForge `/fingerprints` 用十进制 fingerprint;反查侧按字符串比对)。
    ///
    /// 只支持导出反查会用到的三种算法;`Md5` 不参与反查,传入即报错(防误用)。
    pub fn hash(&self, algo: HashAlgo) -> Result<String> {
        match algo {
            HashAlgo::Sha1 => sha1_file(&self.abs),
            HashAlgo::Sha512 => sha512_file(&self.abs),
            HashAlgo::Murmur2 => Ok(cf_fingerprint_file(&self.abs)?.to_string()),
            HashAlgo::Md5 => Err(crate::error::CoreError::other(
                "导出反查不支持 md5 算法(仅 sha1/sha512/murmur2)",
            )),
        }
    }
}

/// 判断一个相对路径(`/` 分隔)是否命中**硬忽略**规则。纯函数,可单测。
///
/// 命中条件(任一):
/// - 任一路径段恰为 [`IGNORED_DIRS`] 之一(目录前缀忽略,连同其下全部内容);
/// - basename 命中 [`IGNORED_FILES`](大小写不敏感);
/// - basename 以 [`IGNORED_SUFFIXES`] 之一结尾(大小写不敏感)。
pub fn is_hard_ignored(rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    // 目录段忽略:任一段命中即整条忽略。
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        if IGNORED_DIRS.iter().any(|d| seg.eq_ignore_ascii_case(d)) {
            return true;
        }
    }
    let base = rel.rsplit('/').next().unwrap_or(&rel);
    let base_lower = base.to_ascii_lowercase();
    if IGNORED_FILES.iter().any(|f| base.eq_ignore_ascii_case(f)) {
        return true;
    }
    if IGNORED_SUFFIXES.iter().any(|s| base_lower.ends_with(s)) {
        return true;
    }
    false
}

/// 判断 `rel` 是否被一组**用户忽略前缀**覆盖(来自 `.packignore`)。
///
/// 前缀语义:`rel == prefix` 或 `rel` 以 `prefix/` 开头都算命中(目录前缀)。前缀里的反斜杠
/// 归一为 `/`,首尾 `/` 去除,空前缀不匹配任何文件(防误把整包排掉)。
pub fn is_user_ignored(rel: &str, prefixes: &[String]) -> bool {
    let rel = rel.replace('\\', "/");
    for p in prefixes {
        let p = p.replace('\\', "/");
        let p = p.trim_matches('/');
        if p.is_empty() {
            continue;
        }
        if rel == p || rel.starts_with(&format!("{p}/")) {
            return true;
        }
    }
    false
}

/// 递归遍历 `game_root`,返回所有**未被忽略**的文件(相对路径稳定升序)。
///
/// `user_ignores` 是来自 `.packignore` 的相对前缀集(可空)。遍历:
/// - 跳过硬忽略目录(进入前剪枝,省去无谓递归);
/// - 经 [`crate::fs::safe_join`] 校验每个相对路径仍落在 `game_root` 内(符号链接逃逸即丢弃);
/// - 不打包目录条目本身(只收文件;空目录不进包);符号链接 / 其它非常规文件跳过。
///
/// 返回的列表按 `rel` 升序排序,使打包 / 索引输出**确定**(便于哈希复现与单测)。
pub fn walk_game_root(game_root: &Path, user_ignores: &[String]) -> Result<Vec<WalkedFile>> {
    let mut out = Vec::new();
    if game_root.is_dir() {
        walk_dir(game_root, game_root, user_ignores, &mut out)?;
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

/// 递归实现:`base` 是 game_root(用于算相对路径),`dir` 是当前目录。
fn walk_dir(
    base: &Path,
    dir: &Path,
    user_ignores: &[String],
    out: &mut Vec<WalkedFile>,
) -> Result<()> {
    use crate::error::IoResultExt;

    for entry in std::fs::read_dir(dir).with_path(dir)? {
        let entry = entry.with_path(dir)?;
        let path = entry.path();
        // 相对 game_root 的路径(`/` 分隔)。
        let rel = match path.strip_prefix(base) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            // 理论上不会发生(path 总在 base 下);保守跳过越权项。
            Err(_) => continue,
        };
        if rel.is_empty() {
            continue;
        }

        // path-safety:相对路径必须能安全收口回 game_root,否则丢弃(防符号链接逃逸)。
        if crate::fs::safe_join(base, &rel).is_none() {
            continue;
        }

        // 硬忽略 / 用户忽略:命中即整条跳过(目录则不再递归)。
        if is_hard_ignored(&rel) || is_user_ignored(&rel, user_ignores) {
            continue;
        }

        let ft = entry.file_type().with_path(&path)?;
        if ft.is_dir() {
            walk_dir(base, &path, user_ignores, out)?;
        } else if ft.is_file() {
            let size = entry.metadata().with_path(&path)?.len();
            out.push(WalkedFile { rel, abs: path, size });
        }
        // 符号链接 / 其它类型不导出(与 zip.rs 打包一致)。
    }
    Ok(())
}

/// 默认门控前缀(Modrinth `accepts` 用):mod / 资源包 / 光影等目录。CurseForge 复用大部分。
pub const MOD_DIR_PREFIXES: &[&str] = &[
    "mods/",
    "coremods/",
    "resourcepacks/",
    "texturepacks/",
    "shaderpacks/",
];

/// 判断 `rel`(`/` 分隔)是否落在 `prefixes` 任一目录前缀下,且扩展名命中 `exts`(不含点,
/// 大小写不敏感)。门控用纯函数,各目标的 `accepts` 直接复用。
pub fn matches_gate(rel: &str, prefixes: &[&str], exts: &[&str]) -> bool {
    let rel = rel.replace('\\', "/");
    if !prefixes.iter().any(|p| rel.starts_with(p)) {
        return false;
    }
    let base = rel.rsplit('/').next().unwrap_or(&rel);
    match base.rsplit_once('.') {
        Some((_, ext)) => exts.iter().any(|e| ext.eq_ignore_ascii_case(e)),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TempRoot {
        path: PathBuf,
    }
    impl TempRoot {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("mc-core-export-walk-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn hard_ignore_covers_runtime_dirs_and_junk() {
        assert!(is_hard_ignored("logs/latest.log"));
        assert!(is_hard_ignored("crash-reports/crash-2020.txt"));
        assert!(is_hard_ignored(".cache/whatever"));
        assert!(is_hard_ignored(".fabric/remappedJars/x.jar"));
        assert!(is_hard_ignored(".quilt/processedMods/y.jar"));
        assert!(is_hard_ignored("mods/.index/sodium.pw.toml"));
        assert!(is_hard_ignored(".DS_Store"));
        assert!(is_hard_ignored("config/.ds_store")); // 大小写不敏感
        assert!(is_hard_ignored("Thumbs.db"));
        assert!(is_hard_ignored("mods/sodium.pw.toml")); // *.pw.toml
    }

    #[test]
    fn hard_ignore_keeps_normal_content() {
        assert!(!is_hard_ignored("mods/sodium.jar"));
        assert!(!is_hard_ignored("config/sodium-options.json"));
        assert!(!is_hard_ignored("options.txt"));
        assert!(!is_hard_ignored("resourcepacks/pack.zip"));
        // 名字里含 "logs" 但不是独立段的不应误伤。
        assert!(!is_hard_ignored("config/dialogs.json"));
        assert!(!is_hard_ignored("mods/catalogue.jar"));
    }

    #[test]
    fn user_ignore_prefix_semantics() {
        let ign = vec!["saves".to_string(), "config/secret.cfg".to_string()];
        assert!(is_user_ignored("saves/world/level.dat", &ign));
        assert!(is_user_ignored("saves", &ign));
        assert!(is_user_ignored("config/secret.cfg", &ign));
        assert!(!is_user_ignored("config/secret.cfg.bak", &ign)); // 非目录前缀,不能前缀误伤
        assert!(!is_user_ignored("mods/a.jar", &ign));
        // 空前缀不匹配任何文件。
        assert!(!is_user_ignored("anything", &["".to_string(), "/".to_string()]));
    }

    #[test]
    fn matches_gate_prefix_and_ext() {
        let exts = &["jar", "litemod", "zip"];
        assert!(matches_gate("mods/sodium.jar", MOD_DIR_PREFIXES, exts));
        assert!(matches_gate("resourcepacks/Faithful.zip", MOD_DIR_PREFIXES, exts));
        assert!(matches_gate("mods/old.LITEMOD", MOD_DIR_PREFIXES, exts)); // 大小写不敏感
        assert!(!matches_gate("config/a.jar", MOD_DIR_PREFIXES, exts)); // 前缀不对
        assert!(!matches_gate("mods/readme.txt", MOD_DIR_PREFIXES, exts)); // 扩展名不对
        assert!(!matches_gate("mods/noext", MOD_DIR_PREFIXES, exts)); // 无扩展名
    }

    #[test]
    fn walk_collects_sorted_and_applies_ignores() {
        let root = TempRoot::new("collect");
        let g = &root.path;
        fs::create_dir_all(g.join("mods")).unwrap();
        fs::write(g.join("mods/b.jar"), b"BBB").unwrap();
        fs::write(g.join("mods/a.jar"), b"AAAA").unwrap();
        fs::create_dir_all(g.join("config/sub")).unwrap();
        fs::write(g.join("config/sub/opts.toml"), b"k=1").unwrap();
        // 硬忽略:logs/ + .DS_Store + *.pw.toml。
        fs::create_dir_all(g.join("logs")).unwrap();
        fs::write(g.join("logs/latest.log"), b"noise").unwrap();
        fs::write(g.join(".DS_Store"), b"x").unwrap();
        fs::create_dir_all(g.join("mods/.index")).unwrap();
        fs::write(g.join("mods/.index/a.pw.toml"), b"meta").unwrap();
        // 用户忽略:saves/。
        fs::create_dir_all(g.join("saves/world")).unwrap();
        fs::write(g.join("saves/world/level.dat"), b"world").unwrap();

        let files = walk_game_root(g, &["saves".to_string()]).unwrap();
        let rels: Vec<&str> = files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            rels,
            vec!["config/sub/opts.toml", "mods/a.jar", "mods/b.jar"],
            "应只剩未忽略文件且按相对路径升序"
        );
        // size 正确。
        let a = files.iter().find(|f| f.rel == "mods/a.jar").unwrap();
        assert_eq!(a.size, 4);
    }

    #[test]
    fn hash_dispatches_by_algo() {
        let root = TempRoot::new("hash");
        let p = root.path.join("x.bin");
        fs::write(&p, b"abc").unwrap();
        let wf = WalkedFile { rel: "x.bin".into(), abs: p, size: 3 };
        // sha1("abc")
        assert_eq!(
            wf.hash(HashAlgo::Sha1).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        // murmur2 走十进制串(与 cf_fingerprint 一致),非空。
        let m = wf.hash(HashAlgo::Murmur2).unwrap();
        assert!(m.parse::<u32>().is_ok());
        // md5 明确拒绝。
        assert!(wf.hash(HashAlgo::Md5).is_err());
    }
}
