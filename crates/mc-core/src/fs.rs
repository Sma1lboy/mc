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

/// Atomically write `data` to `path`: write to a sibling temp file, fsync, then
/// rename over the target. A crash mid-write leaves the old file intact instead
/// of a truncated one. Ports the intent of PrismLauncher's safe `write`.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_path(parent)?;
    }
    // Unique temp name beside the target (same filesystem → rename is atomic).
    // PID alone collides when two writes share a dir within one process — either the
    // same target written concurrently, or two siblings whose extension differs
    // (`a.json` and `a.txt` both → `a.tmp-PID`). A process-global counter makes each
    // call's temp name unique, so concurrent writers never clobber each other's temp.
    static TMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = TMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{}", std::process::id(), seq));

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
mod tests {
    use super::*;

    #[test]
    fn sanitizes_illegal_and_reserved() {
        assert_eq!(sanitize_filename("my/cool:pack", '-'), "my-cool-pack");
        assert_eq!(sanitize_filename("trailing... ", '-'), "trailing");
        assert_eq!(sanitize_filename("CON", '-'), "CON_");
        assert_eq!(sanitize_filename("nul.txt", '-'), "nul.txt_");
        assert_eq!(sanitize_filename("", '-'), "-");
    }

    #[test]
    fn flags_bang_path_as_error() {
        let issues = check_problematic_path(Path::new("/games/cool!/mc"));
        assert!(has_blocking_path_issue(&issues));
    }

    #[test]
    fn flags_non_ascii_as_warning_only() {
        let issues = check_problematic_path(Path::new("/games/我的世界/mc"));
        assert!(!has_blocking_path_issue(&issues));
        assert!(issues.iter().any(|i| i.severity == PathSeverity::Warning));
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize(Path::new("a/b/../c/./d")), PathBuf::from("a/c/d"));
        assert_eq!(path_depth(Path::new("a/b/../c")), 2);
    }

    #[test]
    fn subpath_detection() {
        assert!(is_subpath(Path::new("/root/a/b"), Path::new("/root")));
        assert!(!is_subpath(Path::new("/root/../etc"), Path::new("/root")));
    }

    #[test]
    fn atomic_write_roundtrip() {
        let dir = std::env::temp_dir().join("mc-core-fs-test");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("a/b/c.json");
        write_atomic(&p, b"hello").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "hello");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_siblings_sharing_stem_dont_collide() {
        // a.json / a.txt / ... once shared one temp name (`a.tmp-PID`) and could clobber
        // each other when written concurrently. Each must keep its own content.
        let dir = std::env::temp_dir().join("mc-core-fs-siblings");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exts = ["json", "txt", "cfg", "log", "dat"];
        std::thread::scope(|s| {
            for ext in exts {
                let dir = &dir;
                s.spawn(move || {
                    let p = dir.join(format!("a.{ext}"));
                    write_atomic(&p, ext.as_bytes()).unwrap();
                });
            }
        });
        for ext in exts {
            let p = dir.join(format!("a.{ext}"));
            assert_eq!(std::fs::read_to_string(&p).unwrap(), ext);
        }
        // No temp files left behind.
        let leftover: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftover.is_empty(), "temp files left behind: {leftover:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_join_blocks_traversal() {
        let base = Path::new("/games/mc");
        assert_eq!(safe_join(base, "config/options.txt"), Some(PathBuf::from("/games/mc/config/options.txt")));
        assert_eq!(safe_join(base, "../../etc/passwd"), None);
    }

    #[test]
    fn resolve_segment_rejects_traversal_and_separators() {
        let dir = Path::new("/games/mc/mods");
        // A plain single-segment name resolves to dir/<name>.
        assert_eq!(resolve_segment(dir, "sodium.jar").unwrap(), PathBuf::from("/games/mc/mods/sodium.jar"));
        // Every escape shape is rejected.
        assert!(resolve_segment(dir, "../x").is_err(), "parent-escape must be rejected");
        assert!(resolve_segment(dir, "a/b").is_err(), "embedded separator must be rejected");
        assert!(resolve_segment(dir, "a\\b").is_err(), "backslash separator must be rejected");
        assert!(resolve_segment(dir, "..").is_err(), "'..' must be rejected");
        assert!(resolve_segment(dir, ".").is_err(), "'.' must be rejected");
        assert!(resolve_segment(dir, "").is_err(), "empty segment must be rejected");
    }

    #[test]
    fn is_safe_segment_classifies_names() {
        assert!(is_safe_segment("world1"));
        assert!(is_safe_segment("My Cool Mod.jar"));
        assert!(!is_safe_segment("../x"));
        assert!(!is_safe_segment("a/b"));
        assert!(!is_safe_segment("a\\b"));
        assert!(!is_safe_segment(".."));
        assert!(!is_safe_segment("."));
        assert!(!is_safe_segment(""));
    }

    #[test]
    fn share_file_links_or_copies() {
        let dir = std::env::temp_dir().join("mc-core-share-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.bin");
        std::fs::write(&src, b"data").unwrap();
        let dst = dir.join("store/dst.bin");
        let method = share_file(&src, &dst).unwrap();
        assert!(matches!(method, ShareMethod::HardLink | ShareMethod::Reflink | ShareMethod::Copy));
        assert_eq!(std::fs::read(&dst).unwrap(), b"data");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn override_folder_overlays() {
        let dir = std::env::temp_dir().join("mc-core-override-test");
        let _ = std::fs::remove_dir_all(&dir);
        let ov = dir.join("overrides/config");
        std::fs::create_dir_all(&ov).unwrap();
        std::fs::write(ov.join("a.cfg"), b"x").unwrap();
        let target = dir.join("instance");
        override_folder(&dir.join("overrides"), &target).unwrap();
        assert!(target.join("config/a.cfg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolves_known_executable() {
        // `sh` exists on every unix; on Windows skip (cmd resolution differs).
        if cfg!(unix) {
            assert!(resolve_executable("sh").is_some());
        }
        assert!(resolve_executable("definitely-not-a-real-binary-xyz").is_none());
    }
}
