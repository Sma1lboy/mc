//! Private realms (临时领域) — code-joined, **non-discoverable** shared mod sets.
//!
//! Flow: an owner creates a realm and gets a short join `code`; friends join by
//! code; the owner/admins push a **versioned `manifest`** (the file list to
//! sync); members poll the version and the launcher reconciles each instance to
//! the manifest (the "外侧 syncer"). MVP manifest only carries platform-
//! resolvable files (Modrinth/CurseForge → download url, verified by hash);
//! truly custom jars are surfaced as `manual` for the member to add by hand.
//!
//! Roles: `owner` (creator; disband + role mgmt), `admin` (push manifest),
//! `member` (read + sync). Ids + join codes are minted server-side via
//! `gen_random_uuid()` (present on Supabase / PG13+).

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::session::require_user;
use crate::AppState;

/// One file the syncer must reconcile into a member's instance.
#[derive(Serialize, Deserialize, Clone)]
pub struct RealmFile {
    /// Relative to the instance root, e.g. `mods/sodium.jar`.
    pub path: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub sha512: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Download url (Modrinth/CurseForge). Absent ⇒ `manual` (member adds it).
    #[serde(default)]
    pub url: Option<String>,
    /// `"modrinth"` | `"curseforge"` | `"manual"`.
    #[serde(default)]
    pub source: Option<String>,
}

/// Descriptor for the realm's overrides blob (config/scripts + non-CDN files),
/// stored verbatim in the manifest jsonb; the bytes live on disk (see the
/// `/overrides` endpoints).
#[derive(Serialize, Deserialize, Clone)]
pub struct RealmOverrides {
    pub sha1: String,
    pub size: u64,
}

/// The modpack identity behind the realm (mirrors mc-core's `RealmSource`).
/// Stored verbatim in the manifest jsonb so members keep the modpack source.
#[derive(Serialize, Deserialize, Clone)]
pub struct RealmSource {
    pub provider: String,
    pub project_id: String,
    #[serde(default)]
    pub version_id: Option<String>,
}

/// The versioned sync target an owner/admin publishes. `version` is
/// server-managed (ignored on submit, set on read).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct RealmManifest {
    #[serde(default)]
    pub mc_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub files: Vec<RealmFile>,
    #[serde(default)]
    pub overrides: Option<RealmOverrides>,
    #[serde(default)]
    pub source: Option<RealmSource>,
    #[serde(default)]
    pub version: i32,
}

/// A realm as seen by a member (includes *their* role).
#[derive(Serialize, Clone)]
pub struct RealmSummary {
    pub id: String,
    pub code: String,
    pub name: String,
    pub owner_id: String,
    pub mc_version: Option<String>,
    pub loader: Option<String>,
    pub manifest_version: i32,
    pub role: String,
    pub expires_at: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateRealmReq {
    pub name: String,
    /// Seconds until the realm expires; `None`/`0` ⇒ no expiry.
    #[serde(default)]
    pub expires_in_secs: Option<i64>,
    #[serde(default)]
    pub manifest: RealmManifest,
}

#[derive(Deserialize)]
pub struct JoinReq {
    pub code: String,
}

#[derive(Deserialize)]
pub struct RoleReq {
    /// `"admin"` | `"member"`.
    pub role: String,
}

#[derive(Deserialize)]
pub struct SyncedReq {
    pub version: i32,
}

#[derive(Serialize)]
pub struct MemberInfo {
    pub user_id: String,
    pub username: Option<String>,
    pub role: String,
    pub synced_version: i32,
    pub joined_at: Option<String>,
}

mod handlers;
mod store;
#[cfg(test)]
mod tests;

pub use handlers::*;
pub use store::*;
