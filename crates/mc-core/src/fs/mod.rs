//! Filesystem & path-handling utilities ported from the reference launchers'
//! battle-tested logic (PrismLauncher `FileSystem.cpp`, PCL path pre-checks).
//!
//! These cover the gritty real-world cases our basic [`crate::paths`] layout
//! helpers don't: sanitising user text into safe folder names, detecting paths
//! that silently break Java/Minecraft (the infamous `!` gotcha, non-ASCII paths),
//! crash-safe atomic writes, and lexical path normalisation.

use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, IoResultExt, Result};

/// Characters that are illegal in a filename on at least one supported OS.
const INVALID_FILENAME_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Windows reserved device names (case-insensitive, with or without extension).
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Turn arbitrary user text into a filename that is safe on Windows/macOS/Linux.
///
/// Ports PrismLauncher's `RemoveInvalidFilenameChars` + reserved-name handling:
/// illegal and control characters become `replacement`, trailing dots/spaces are
/// stripped (Windows trims them, causing surprises), reserved device names get a
/// `_` suffix, and an empty result falls back to `_`.
pub fn sanitize_filename(input: &str, replacement: char) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if INVALID_FILENAME_CHARS.contains(&ch) || (ch as u32) < 0x20 {
            out.push(replacement);
        } else {
            out.push(ch);
        }
    }

    // Windows silently strips trailing dots and spaces — do it explicitly so the
    // name we store matches the name on disk.
    let trimmed = out.trim_end_matches([' ', '.']).to_string();
    let mut result = if trimmed.is_empty() { replacement.to_string() } else { trimmed };

    // Avoid reserved device names (compare the stem, case-insensitively).
    let stem = result.split('.').next().unwrap_or(&result).to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        result.push('_');
    }
    if result.is_empty() {
        result.push('_');
    }
    result
}

/// 把展示名清洗成一个文件系统安全的**目录**名:路径分隔符 / 保留字 / 控制符 / 空白都
/// 归一为单个 `-`,去掉首尾 `-`;**保留 unicode**(中文名可直接作目录名)。空结果回退
/// `fallback`。这是「展示名 → 安全目录名」的唯一 owner——实例 id 与世界文件夹共用同一套
/// 规则(此前两处逐字符重复,只差回退串)。
///
/// 注意与 [`sanitize_filename`] 的分工:那个面向**任意文件名**(可配替换字符、处理 Windows
/// 保留设备名与尾随点),这个面向**目录名**(空白折叠成 `-`、保留 unicode、可配空回退)。
pub fn slugify(name: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.trim().chars() {
        let bad = ch.is_whitespace()
            || ch.is_control()
            || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
        if bad {
            if !prev_dash && !out.is_empty() {
                out.push('-');
                prev_dash = true;
            }
        } else {
            out.push(ch);
            prev_dash = false;
        }
    }
    let s = out.trim_matches('-').to_string();
    if s.is_empty() {
        fallback.to_string()
    } else {
        s
    }
}

/// 给 `base` 找一个不冲突的名字:`base` 本身可用(`exists` 为 false)就用它,否则依次试
/// `base-2`/`base-3`… 直到不冲突。`exists` 抽象掉「在哪检查冲突」——实例目录用
/// `version_dir(c).exists()`,世界文件夹用 `saves.join(c).exists()`,共用同一套后缀逻辑。
pub fn unique_name(base: &str, mut exists: impl FnMut(&str) -> bool) -> String {
    if !exists(base) {
        return base.to_string();
    }
    (2u32..)
        .map(|n| format!("{base}-{n}"))
        .find(|cand| !exists(cand))
        .unwrap_or_else(|| base.to_string())
}

/// Build a unique directory name for `name` inside `parent`, sanitising and then
/// appending `-2`, `-3`, … if a folder with that name already exists.
///
/// Ports PrismLauncher's `DirNameFromString`.
pub fn dir_name_from_string(name: &str, parent: &Path) -> String {
    let base = sanitize_filename(name, '-');
    let mut candidate = base.clone();
    let mut n = 1;
    while parent.join(&candidate).exists() {
        n += 1;
        candidate = format!("{base}-{n}");
    }
    candidate
}

/// Severity of a [`PathIssue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSeverity {
    /// Will almost certainly break launching — refuse or strongly warn.
    Error,
    /// Known to cause problems for some versions/mods — warn the user.
    Warning,
}

/// A problem found in a game/instance path.
#[derive(Debug, Clone)]
pub struct PathIssue {
    pub severity: PathSeverity,
    pub message: String,
}

/// Inspect a path for characters/patterns known to break Java or Minecraft.
///
/// This is the cross-launcher folklore distilled (PrismLauncher
/// `checkProblemticPathJava`, PCL's `McLaunchPrecheck`):
///
/// - `!` — Java treats it as the jar-URL separator; classpath entries under such
///   a path fail to load. This is a hard error.
/// - non-ASCII (e.g. Chinese) — some older Forge/OptiFine builds and a few native
///   loaders mis-handle it; modern MC is usually fine, so it's a warning.
/// - whitespace — generally fine now but historically fragile; informational warn.
pub fn check_problematic_path(path: &Path) -> Vec<PathIssue> {
    let s = path.to_string_lossy();
    let mut issues = Vec::new();

    if s.contains('!') {
        issues.push(PathIssue {
            severity: PathSeverity::Error,
            message: "路径包含 '!',会破坏 Java 的 classpath 解析,请把游戏目录移到不含 '!' 的路径。".into(),
        });
    }
    if !s.is_ascii() {
        issues.push(PathIssue {
            severity: PathSeverity::Warning,
            message: "路径包含非 ASCII 字符(如中文),部分老版本 Forge/OptiFine 或原生库可能出错。".into(),
        });
    }
    if s.contains(' ') {
        issues.push(PathIssue {
            severity: PathSeverity::Warning,
            message: "路径包含空格,极少数旧版本/Mod 可能受影响。".into(),
        });
    }
    issues
}

/// True if any issue is an [`PathSeverity::Error`].
pub fn has_blocking_path_issue(issues: &[PathIssue]) -> bool {
    issues.iter().any(|i| i.severity == PathSeverity::Error)
}

/// A unique sibling temp path for an atomic replace of `path`: same directory (so
/// the follow-up `rename` stays on one filesystem and is atomic), with a name no
/// concurrent writer in *this process* can collide on. The original filename is
/// kept and `.<tag>-<pid>-<seq>` appended, so temps read clearly on disk
/// (`foo.jar.part-…` for a streamed download, `cfg.json.tmp-…` for an atomic write).
///
/// Uniqueness is keyed by a process-global counter, NOT by the destination: two
/// writers racing to replace the *same* path — e.g. two instances installing the
/// same library into the shared `libraries/` store — get distinct temps, so
/// neither truncates the other's bytes nor deletes its in-progress file on a
/// verify-fail. This is the one owner of "temp name for an atomic file replace";
/// both [`write_atomic`] and the download engine route through it.
pub fn unique_temp_sibling(path: &Path, tag: &str) -> PathBuf {
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.to_path_buf().into_os_string();
    name.push(format!(".{tag}-{}-{}", std::process::id(), seq));
    PathBuf::from(name)
}

/// Atomically write `data` to `path`: write to a sibling temp file, fsync, then
/// rename over the target. A crash mid-write leaves the old file intact instead
/// of a truncated one. Ports the intent of PrismLauncher's safe `write`.
///
/// **Invariant — creates `path`'s parent directory if missing.** Callers need not
/// `ensure_dir(path.parent())` before writing; the atomic write owns that. (Every
/// `write_atomic` caller relies on this rather than re-`mkdir`-ing first.)
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    let tmp = unique_temp_sibling(path, "tmp");

    {
        let mut f = std::fs::File::create(&tmp).with_path(&tmp)?;
        f.write_all(data).with_path(&tmp)?;
        f.sync_all().with_path(&tmp)?;
    }
    std::fs::rename(&tmp, path).with_path(path).inspect_err(|_| {
        // Best-effort cleanup of the temp file on failure.
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Lexically normalise a path: resolve `.` and `..` and collapse separators
/// without touching the filesystem (so it works on not-yet-created paths).
/// Ports PrismLauncher's `NormalizePath` intent.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                // Pop a normal segment; keep `..` if there's nothing to pop.
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Number of path segments (ignoring the root / prefix). Ports `pathDepth`.
pub fn path_depth(path: &Path) -> usize {
    normalize(path)
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count()
}

/// True if `child` is `base` or lives underneath it, compared lexically after
/// normalisation. Useful to keep operations inside a game root (no `..` escapes).
pub fn is_subpath(child: &Path, base: &Path) -> bool {
    normalize(child).starts_with(normalize(base))
}

/// Resolve an executable: an absolute/relative path is returned if it exists and
/// is a file; a bare name is searched on `PATH` (`PATHEXT` on Windows). Ports
/// PrismLauncher's `ResolveExecutable`. Returns `None` if nothing usable found.
pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }

    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.BAT;.CMD".into())
            .split(';')
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![String::new()]
    };

    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let cand = dir.join(format!("{name}{ext}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}

/// Move `src` to `dst`, falling back to copy-then-delete when a plain rename
/// fails (e.g. across filesystems). Ports `move`/`moveByCopy`.
pub fn move_with_fallback(src: &Path, dst: &Path) -> Result<()> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    // Cross-device or other rename failure: copy then remove.
    if src.is_dir() {
        copy_dir(src, dst)?;
        std::fs::remove_dir_all(src).with_path(src)?;
    } else {
        std::fs::copy(src, dst).with_path(dst)?;
        std::fs::remove_file(src).with_path(src)?;
    }
    Ok(())
}

/// Delete `path`, preferring the OS recycle bin so the user can recover it.
///
/// Try [`trash::delete`] first (reversible); only when that fails — headless /
/// containerised hosts with no trash backend, CI — fall back to an irreversible
/// in-place removal, choosing [`remove_dir_all`](std::fs::remove_dir_all) for a
/// directory and [`remove_file`](std::fs::remove_file) otherwise. IO errors on
/// the hard-delete fallback carry the offending path via [`IoResultExt`].
///
/// The one owner of "trash, else hard-delete (dir vs file)". Every resource
/// delete — mods, packs, screenshots, worlds, instances — routes through it.
/// Callers keep their own `if !path.exists() { return Ok(()) }` idempotence
/// guard before calling (the not-found / path-resolution semantics differ per
/// module); this helper assumes `path` is the resolved thing to remove.
pub fn trash_or_delete(path: &Path) -> Result<()> {
    if trash::delete(path).is_ok() {
        return Ok(());
    }
    // Trash unavailable: irreversible removal, branching on what's on disk.
    if path.is_dir() {
        std::fs::remove_dir_all(path).with_path(path)
    } else {
        std::fs::remove_file(path).with_path(path)
    }
}

/// Walk up until an existing ancestor is found (for stat-ing a not-yet-created
/// path). Ports `nearestExistentAncestor`.
pub fn nearest_existent_ancestor(path: &Path) -> Option<PathBuf> {
    let mut cur = normalize(path);
    loop {
        if cur.exists() {
            return Some(cur);
        }
        if !cur.pop() {
            return None;
        }
    }
}

/// Bytes available on the filesystem holding `path` (or its nearest existing
/// ancestor). Enables a disk-space pre-check before a multi-GB install.
pub fn available_space(path: &Path) -> Result<u64> {
    let target = nearest_existent_ancestor(path).unwrap_or_else(|| PathBuf::from("."));
    fs4::available_space(&target).map_err(|e| crate::error::CoreError::io(target, e))
}

/// How a file was placed into a shared store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareMethod {
    /// Same inode — zero extra space (best for immutable library/asset stores).
    HardLink,
    /// Copy-on-write clone (APFS/Btrfs/XFS/ReFS) — zero space until divergence.
    Reflink,
    /// Plain byte copy (fallback when neither is supported / cross-device).
    Copy,
}

/// Place `src` at `dst` as cheaply as the filesystem allows: try a hard link
/// (true sharing, ideal for the immutable libraries/assets store), then a
/// reflink (copy-on-write), then a plain copy. This is how multiple instances
/// share one library/asset store without N× disk usage.
pub fn share_file(src: &Path, dst: &Path) -> Result<ShareMethod> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    if dst.exists() {
        let _ = std::fs::remove_file(dst);
    }
    if std::fs::hard_link(src, dst).is_ok() {
        return Ok(ShareMethod::HardLink);
    }
    if reflink_copy::reflink(src, dst).is_ok() {
        return Ok(ShareMethod::Reflink);
    }
    std::fs::copy(src, dst).with_path(dst)?;
    Ok(ShareMethod::Copy)
}

/// Recursively copy `src` directory into `dst`, overwriting existing files.
/// The building block for modpack `overrides` and the move fallback.
pub fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).with_path(dst)?;
    for entry in std::fs::read_dir(src).with_path(src)? {
        let entry = entry.with_path(src)?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p).with_path(p)?;
            }
            std::fs::copy(&from, &to).with_path(&to)?;
        }
    }
    Ok(())
}

/// Overlay the contents of `override_dir` onto `target` (modpack `overrides`).
/// Ports `overrideFolder`.
pub fn override_folder(override_dir: &Path, target: &Path) -> Result<()> {
    if !override_dir.is_dir() {
        return Ok(());
    }
    copy_dir(override_dir, target)
}

/// True if `segment` is a single, inert path component — not empty, not `.` or
/// `..`, and containing no `/` or `\` separator. The one check that keeps a
/// caller- or platform-supplied name (a mod/world/pack filename, a Modrinth
/// `filename`) from escaping the directory it's meant to live in.
///
/// Unlike [`safe_join`] (which lexically resolves a multi-segment archive path
/// against a base), this rejects *any* path structure outright: the inputs here
/// are meant to be a bare file/folder name, so `a/b` is just as illegal as
/// `../x`. This is the centralised guard behind [`resolve_segment`].
pub fn is_safe_segment(segment: &str) -> bool {
    !(segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.contains('/')
        || segment.contains('\\'))
}

/// Validate that `segment` is a single inert path component (see
/// [`is_safe_segment`]) and, if so, return `dir.join(segment)`. Rejects a bad
/// segment (`../x`, `a/b`, `..`, `.`, empty) with a [`CoreError`] so a frontend-
/// or platform-supplied name can never reach outside `dir`.
///
/// This is the one shared single-segment validator; modules that locate a file
/// by an externally-supplied name route through it instead of joining raw.
pub fn resolve_segment(dir: &Path, segment: &str) -> Result<PathBuf> {
    if !is_safe_segment(segment) {
        return Err(CoreError::other(format!("非法路径段: {segment}")));
    }
    Ok(dir.join(segment))
}

/// Safely join an archive-internal relative path under `base`, refusing any
/// result that escapes `base` (zip-slip / path-traversal guard). Returns `None`
/// for a malicious entry like `../../etc/passwd`.
pub fn safe_join(base: &Path, relative: &str) -> Option<PathBuf> {
    let joined = normalize(&base.join(relative));
    is_subpath(&joined, base).then_some(joined)
}

/// Generate a minimal `launcher_profiles.json` in a game root if absent. The
/// vanilla Forge/legacy installers refuse to run without this file; ports the
/// PCL `launcher_profiles.json` generation. Existing files are left untouched.
pub fn ensure_launcher_profiles(root: &Path) -> Result<()> {
    let path = root.join("launcher_profiles.json");
    if path.exists() {
        return Ok(());
    }
    let content = r#"{
  "profiles": {},
  "selectedProfile": "",
  "clientToken": "",
  "authenticationDatabase": {},
  "launcherVersion": { "name": "mc-launcher", "format": 21 },
  "settings": {}
}
"#;
    write_atomic(&path, content.as_bytes())
}

#[cfg(test)]
mod tests;
