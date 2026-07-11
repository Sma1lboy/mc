//! Private realms (临时领域) — client + the "外侧 syncer".
//!
//! A realm is a code-joined, non-discoverable shared mod set hosted by
//! [`mc-server`](crate::server). The owner/admins publish a versioned
//! **manifest** (the file list to sync); members reconcile an instance to it.
//!
//! This module holds three things:
//! 1. **DTOs** mirroring `mc-server`'s realm wire shapes (so they flow to the
//!    desktop bindings via `specta`).
//! 2. **Client methods** on [`ServerClient`](crate::server::ServerClient) for the
//!    `/v1/realms/*` endpoints.
//! 3. The **syncer**: [`plan_sync`] computes what a sync would change without
//!    touching disk (removals come only from the [`SyncState`] ledger — files
//!    the member added themselves are never deleted); [`apply_sync`] downloads
//!    the missing/changed files, (optionally) removes mods the manifest dropped,
//!    and records the ledger; [`build_manifest_from_instance`]
//!    turns a host's instance into a manifest by resolving each local mod jar to
//!    a platform download url by hash (unresolvable jars are surfaced as `manual`).
//!
//! Scope: the platform-resolvable content dirs ([`MANAGED_DIRS`] — mods,
//! resourcepacks, shaderpacks, datapacks) are reconciled by url+hash. Files the
//! provider doesn't recognise are surfaced as `manual` for now (Phase 2 bundles
//! them — configs/scripts included — into a separate overrides blob).

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::download::checksum::{verify_sha1, verify_sha512};
use crate::download::{DownloadItem, Downloader};
use crate::error::{CoreError, Result};
use crate::instance::Instance;
use crate::modpack::export::walk::walk_game_root;
use crate::modplatform::provider::ResourceProvider;
use crate::modplatform::{HashAlgo, ProviderId};
use crate::server::ServerClient;
use crate::types::Progress;

/// Instance subdirectories the syncer reconciles to the manifest. These are the
/// platform-resolvable content dirs (files get a CDN url by hash). Override-only
/// dirs (`config`/`scripts`/`kubejs`) ride a separate blob — see the overrides flow.
const MANAGED_DIRS: &[&str] = &["mods", "resourcepacks", "shaderpacks", "datapacks"];

/* ---------- wire DTOs (mirror crates/mc-server/src/realm.rs) ---------- */

/// One file the syncer reconciles into a member's instance. `path` is relative
/// to the **instance dir**, e.g. `mods/sodium.jar`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RealmFile {
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

/// Non-CDN files (config/scripts + unresolved content) bundled as one zip blob,
/// stored on the server and fetched by members. The manifest carries only this
/// descriptor; the bytes live behind `GET /v1/realms/{id}/overrides`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RealmOverrides {
    /// sha1 of the zip — integrity + "did it change since I applied it" check.
    pub sha1: String,
    pub size: u64,
}

/// The modpack identity behind the realm (when the host's instance was installed
/// from a provider modpack). Carried so members' synced instances keep the
/// modpack source — their instance detail can then show the modpack overview,
/// not just a bare instance. Mirrors [`crate::instance::config::InstanceSource`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RealmSource {
    pub provider: String,
    pub project_id: String,
    #[serde(default)]
    pub version_id: Option<String>,
}

/// The versioned sync target. `version` is server-managed (set on read).
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct RealmManifest {
    #[serde(default)]
    pub mc_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub files: Vec<RealmFile>,
    /// The overrides blob descriptor, if the snapshot has non-CDN files.
    #[serde(default)]
    pub overrides: Option<RealmOverrides>,
    /// The modpack identity (if the host's instance came from a provider modpack),
    /// so members keep the source on their synced instance (icon rides the blob).
    #[serde(default)]
    pub source: Option<RealmSource>,
    #[serde(default)]
    pub version: i32,
}

/// A realm as seen by a member (includes *their* role).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RealmSummary {
    pub id: String,
    pub code: String,
    pub name: String,
    pub owner_id: String,
    #[serde(default)]
    pub mc_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    pub manifest_version: i32,
    pub role: String,
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// One realm member + how far they've synced.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RealmMember {
    pub user_id: String,
    #[serde(default)]
    pub username: Option<String>,
    pub role: String,
    pub synced_version: i32,
    #[serde(default)]
    pub joined_at: Option<String>,
}

/// Body for `POST /v1/realms` (create).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CreateRealmReq {
    pub name: String,
    /// Seconds until expiry; `None`/`0` ⇒ no expiry.
    #[serde(default)]
    pub expires_in_secs: Option<i64>,
    #[serde(default)]
    pub manifest: RealmManifest,
}

#[derive(Serialize)]
struct JoinBody {
    code: String,
}
#[derive(Serialize)]
struct RoleBody {
    role: String,
}
#[derive(Serialize)]
struct SyncedBody {
    version: i32,
}
#[derive(Serialize)]
struct InviteBody {
    user_id: String,
}
#[derive(Serialize)]
struct SetHostBody {
    address: String,
}
#[derive(Deserialize)]
struct VersionResp {
    version: i32,
}

/// Who is currently hosting a realm's LAN-opened world. Both fields are `None`
/// when nobody is hosting (or the host's heartbeat went stale). `address` is
/// `<virtual_ip>:<lan_port>` — pass it straight to Quick Play to join.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct RealmHost {
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub host_username: Option<String>,
}

mod api;
mod sync;
#[cfg(test)]
mod tests;

pub use sync::*;
