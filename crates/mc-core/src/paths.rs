//! Directory model and portability. Implements `docs/07-directory-model-portability.md`:
//! the launcher binary lives outside every instance, but discovers game roots by
//! scanning its own directory (portable), the OS-official `.minecraft`, and
//! user-added custom roots.

use std::path::{Path, PathBuf};

use mc_types::{GameRoot, RootKind};

use crate::error::{IoResultExt, Result};

/// Resolve where the launcher should keep its own data (accounts, settings,
/// downloaded Java, caches).
///
/// If a portable marker (`portable.txt` or `.portable`) sits next to the
/// executable, data is kept beside the exe so the whole folder is movable;
/// otherwise it goes to the OS application-data directory.
pub fn resolve_data_dir(exe_dir: &Path) -> PathBuf {
    if exe_dir.join("portable.txt").exists() || exe_dir.join(".portable").exists() {
        exe_dir.join("launcher-data")
    } else {
        dirs::data_dir()
            .map(|d| d.join("mc-launcher"))
            .unwrap_or_else(|| exe_dir.join("launcher-data"))
    }
}

/// 全局日志目录:`<data_dir>/logs`。client(前端)与 daemon(本地数据层)的日志统一写这里,
/// 与游戏根目录无关,便于跨实例排查问题。
pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

/// The OS's official Minecraft directory, if the platform has a conventional one.
pub fn official_minecraft_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        dirs::data_dir().map(|d| d.join(".minecraft"))
    }
    #[cfg(target_os = "macos")]
    {
        dirs::data_dir().map(|d| d.join("minecraft"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        dirs::home_dir().map(|d| d.join(".minecraft"))
    }
}

/// Decide whether `dir` (or `dir/.minecraft`) looks like a Minecraft game root.
///
/// The check is intentionally cheap — it only inspects directory structure and
/// never parses any json. A root is recognised when it contains a `versions/`
/// subdirectory.
pub fn detect_game_root(dir: &Path) -> Option<PathBuf> {
    [dir.to_path_buf(), dir.join(".minecraft")]
        .into_iter()
        .find(|cand| cand.join("versions").is_dir())
}

fn root_name(path: &Path, kind: RootKind) -> String {
    match kind {
        RootKind::Official => "官方目录".to_string(),
        RootKind::Default => "默认".to_string(),
        _ => path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
    }
}

fn make_root(path: PathBuf, kind: RootKind) -> GameRoot {
    GameRoot { name: root_name(&path, kind), path: path.to_string_lossy().into_owned(), kind }
}

/// Discover all game roots at startup, in priority order:
/// 1. portable (next to the exe), 2. official, 3. user custom, 4. default fallback.
///
/// `custom` are user-added paths loaded from settings. Duplicate paths are
/// collapsed, keeping the highest-priority kind.
pub fn discover_roots(exe_dir: &Path, data_dir: &Path, custom: &[PathBuf]) -> Vec<GameRoot> {
    let mut roots: Vec<GameRoot> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    let push = |path: PathBuf, kind: RootKind, roots: &mut Vec<GameRoot>, seen: &mut Vec<PathBuf>| {
        let canon = path.canonicalize().unwrap_or_else(|_| path.clone());
        if seen.iter().any(|p| p == &canon) {
            return;
        }
        seen.push(canon);
        roots.push(make_root(path, kind));
    };

    // 1. portable / sibling detection
    if let Some(p) = detect_game_root(exe_dir) {
        push(p, RootKind::Portable, &mut roots, &mut seen);
    }
    // 2. official
    if let Some(p) = official_minecraft_dir() {
        if p.join("versions").is_dir() {
            push(p, RootKind::Official, &mut roots, &mut seen);
        }
    }
    // 3. user custom
    for p in custom {
        if p.is_dir() {
            push(p.clone(), RootKind::Custom, &mut roots, &mut seen);
        }
    }
    // 4. fallback
    if roots.is_empty() {
        roots.push(make_root(data_dir.join(".minecraft"), RootKind::Default));
    }

    roots
}

/// Filesystem layout of a single game root. All instance paths derive from here.
#[derive(Debug, Clone)]
pub struct GamePaths {
    root: PathBuf,
}

impl GamePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn version_dir(&self, id: &str) -> PathBuf {
        self.versions_dir().join(id)
    }

    pub fn version_json(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }

    pub fn version_jar(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn asset_indexes_dir(&self) -> PathBuf {
        self.assets_dir().join("indexes")
    }

    pub fn asset_objects_dir(&self) -> PathBuf {
        self.assets_dir().join("objects")
    }

    /// Content-addressed asset object path: `objects/<hash[0..2]>/<hash>`.
    pub fn asset_object(&self, hash: &str) -> PathBuf {
        self.asset_objects_dir().join(&hash[0..2.min(hash.len())]).join(hash)
    }

    /// Legacy (pre-1.6) virtual assets directory for a given index.
    pub fn assets_virtual_dir(&self, index_id: &str) -> PathBuf {
        self.assets_dir().join("virtual").join(index_id)
    }

    /// Per-version natives extraction directory.
    pub fn natives_dir(&self, id: &str) -> PathBuf {
        self.version_dir(id).join("natives")
    }
}

/// Create a directory and all parents, attaching the path to any error.
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).with_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_paths_layout() {
        let p = GamePaths::new("/games/mc");
        assert_eq!(p.version_json("1.20.1"), PathBuf::from("/games/mc/versions/1.20.1/1.20.1.json"));
        assert_eq!(
            p.asset_object("abcdef0123"),
            PathBuf::from("/games/mc/assets/objects/ab/abcdef0123")
        );
    }

    #[test]
    fn discover_falls_back_to_default() {
        let tmp = std::env::temp_dir().join("mc-core-test-empty-root");
        let roots = discover_roots(&tmp, &tmp, &[]);
        assert!(!roots.is_empty());
        assert_eq!(roots.last().unwrap().kind, RootKind::Default);
    }
}
