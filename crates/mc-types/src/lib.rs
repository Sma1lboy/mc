//! `mc-types` — plain, logic-free data types shared between `mc-core`, the CLI and
//! the desktop UI. Everything here is `serde`-serializable so it can cross the
//! Tauri IPC boundary unchanged. No behaviour lives in this crate.

use serde::{Deserialize, Serialize};

pub mod platform;

pub use platform::{Arch, Os, Platform};

/// Progress report emitted by long-running tasks (download, verify, launch).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct Progress {
    /// Human-readable description of the current stage, e.g. "下载 libraries".
    pub stage: String,
    /// Units completed so far.
    pub current: u64,
    /// Total units (0 if unknown).
    pub total: u64,
    /// Instantaneous speed in bytes/sec (0 if not applicable).
    pub speed_bps: u64,
}

impl Progress {
    pub fn new(stage: impl Into<String>) -> Self {
        Self { stage: stage.into(), current: 0, total: 0, speed_bps: 0 }
    }

    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            (self.current as f64 / self.total as f64).clamp(0.0, 1.0)
        }
    }
}

/// How a game-root directory was discovered. See `docs/07-directory-model-portability.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum RootKind {
    /// Detected next to / inside the launcher executable directory (portable).
    Portable,
    /// The OS's official `.minecraft` location.
    Official,
    /// A directory the user added manually.
    Custom,
    /// Auto-created fallback under the launcher data directory.
    Default,
}

/// A `.minecraft`-style game directory that holds versions / libraries / assets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct GameRoot {
    pub name: String,
    pub path: String,
    pub kind: RootKind,
}

/// The three kinds of account the launcher supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    Offline,
    Microsoft,
    Yggdrasil,
}

/// The authenticated identity handed to the launch pipeline. This is the single
/// exit point all account kinds funnel into — the launch code never branches on
/// account type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct AuthSession {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    /// `msa` or `legacy`, passed to the game as `${user_type}`.
    pub user_type: String,
    /// Xbox user id (`${auth_xuid}`), empty for non-Microsoft accounts.
    #[serde(default)]
    pub xuid: String,
}

/// A persisted account as shown in the account switcher.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct AccountSummary {
    pub kind: AccountKind,
    pub username: String,
    pub uuid: String,
    /// True for the currently selected account.
    #[serde(default)]
    pub selected: bool,
    /// Whether the account owns Minecraft (Microsoft accounts only).
    #[serde(default)]
    pub owns_game: bool,
}

/// Mod loader families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Vanilla,
    Forge,
    NeoForge,
    Fabric,
    Quilt,
    LiteLoader,
    OptiFine,
}

impl LoaderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LoaderKind::Vanilla => "vanilla",
            LoaderKind::Forge => "forge",
            LoaderKind::NeoForge => "neoforge",
            LoaderKind::Fabric => "fabric",
            LoaderKind::Quilt => "quilt",
            LoaderKind::LiteLoader => "liteloader",
            LoaderKind::OptiFine => "optifine",
        }
    }

    /// The exact inverse of [`as_str`](Self::as_str): parse a loader-family name
    /// (case-insensitive, surrounding whitespace ignored) back into a `LoaderKind`,
    /// or `None` if it isn't a known family. This is THE one owner of "family string
    /// → kind"; every parser (realm stubs, modpack import, export commands) routes
    /// through it instead of re-listing the arms and drifting (e.g. silently
    /// dropping `liteloader`/`optifine` into a Vanilla bucket).
    pub fn from_family(s: &str) -> Option<LoaderKind> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "vanilla" => LoaderKind::Vanilla,
            "forge" => LoaderKind::Forge,
            "neoforge" => LoaderKind::NeoForge,
            "fabric" => LoaderKind::Fabric,
            "quilt" => LoaderKind::Quilt,
            "liteloader" => LoaderKind::LiteLoader,
            "optifine" => LoaderKind::OptiFine,
            _ => return None,
        })
    }
}

/// The release channel of a Minecraft version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseKind {
    Release,
    Snapshot,
    OldBeta,
    OldAlpha,
}

/// One entry from Mojang's version manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct ManifestVersion {
    pub id: String,
    pub kind: ReleaseKind,
    pub url: String,
    /// SHA1 of the version json (Mojang manifest_v2 provides this).
    #[serde(default)]
    pub sha1: String,
    /// ISO-8601 release time.
    #[serde(default)]
    pub release_time: String,
}

/// The realm an instance belongs to (临时领域), stored on the instance and
/// surfaced for badges + the in-instance realm panel. An instance maps to at
/// most one realm (1:1). For a freshly-joined instance the core isn't installed
/// yet — [`InstanceSummary::installed`] is false until the user hits "begin".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct RealmRef {
    pub realm_id: String,
    /// Join code (for display / re-share).
    #[serde(default)]
    pub code: Option<String>,
    /// This client's role in the realm: `owner` | `admin` | `member`.
    pub role: String,
    /// Realm display name (cached for offline display).
    #[serde(default)]
    pub name: Option<String>,
    /// Target Minecraft version (so a pending instance can install on "begin").
    #[serde(default)]
    pub mc_version: Option<String>,
    /// Target loader (`fabric` etc.) for the pending install.
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
}

/// A summary of an instance for list views.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct InstanceSummary {
    pub id: String,
    pub name: String,
    /// The base Minecraft version, e.g. "1.20.1".
    pub mc_version: String,
    pub loader: LoaderKind,
    /// Loader version if applicable.
    #[serde(default)]
    pub loader_version: Option<String>,
    /// Relative icon path or builtin icon key.
    #[serde(default)]
    pub icon: Option<String>,
    /// Epoch millis of last launch, 0 if never.
    #[serde(default)]
    pub last_played: u64,
    /// Whether the instance is currently running.
    #[serde(default)]
    pub running: bool,
    /// Whether the core (version + loader) is installed. A realm instance that's
    /// been joined but not yet synced is `false` — it can't launch until "begin".
    #[serde(default = "default_true")]
    pub installed: bool,
    /// The realm this instance belongs to, if any (host = `owner`).
    #[serde(default)]
    pub realm: Option<RealmRef>,
    /// Free-form user tags for grouping / filtering in the Library.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// UI theme preference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, specta::Type)]
pub struct ThemeConfig {
    /// "dark" or "light".
    pub mode: String,
    /// Accent hue 0-360.
    pub hue: f64,
    /// Accent saturation 0-100.
    pub saturation: f64,
    /// Accent lightness 0-100.
    pub lightness: f64,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        // Modrinth-ish green default, see docs/06.
        Self { mode: "dark".into(), hue: 150.0, saturation: 60.0, lightness: 45.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_kind_from_family_is_exact_inverse_of_as_str() {
        use LoaderKind::*;
        // Every variant round-trips through as_str -> from_family.
        for k in [Vanilla, Forge, NeoForge, Fabric, Quilt, LiteLoader, OptiFine] {
            assert_eq!(LoaderKind::from_family(k.as_str()), Some(k));
        }
        // Case- and whitespace-insensitive.
        assert_eq!(LoaderKind::from_family("  NeoForge "), Some(NeoForge));
        // liteloader/optifine were the families the realm parser used to drop into
        // the Vanilla bucket — they must resolve to themselves now.
        assert_eq!(LoaderKind::from_family("liteloader"), Some(LiteLoader));
        assert_eq!(LoaderKind::from_family("optifine"), Some(OptiFine));
        // Unknown families are None (callers decide their own default).
        assert_eq!(LoaderKind::from_family("rift"), None);
        assert_eq!(LoaderKind::from_family(""), None);
    }
}
