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
//!    touching disk; [`apply_sync`] downloads the missing/changed files and
//!    (optionally) removes mods the manifest dropped; [`build_manifest_from_instance`]
//!    turns a host's instance into a manifest by resolving each local mod jar to
//!    a platform download url by hash (unresolvable jars are surfaced as `manual`).
//!
//! MVP scope: only the `mods/` directory is reconciled, and only Modrinth-
//! resolvable (or already-hashed) files carry a download url — truly custom jars
//! are `manual` and the member adds them by hand.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::download::checksum::{sha1_file, verify_sha1, verify_sha512};
use crate::download::{DownloadItem, Downloader};
use crate::error::{CoreError, Result};
use crate::instance::{list_mods, Instance};
use crate::modplatform::provider::ResourceProvider;
use crate::modplatform::{HashAlgo, ProviderId};
use crate::server::ServerClient;
use crate::types::Progress;

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
#[derive(Deserialize)]
struct VersionResp {
    version: i32,
}

/* ---------- client methods ---------- */

impl ServerClient {
    /// Create a realm (the caller must be logged in); returns the new summary.
    pub async fn create_realm(&self, req: &CreateRealmReq) -> Result<RealmSummary> {
        self.post_json("/v1/realms", req).await
    }

    /// Join by code. `Ok(None)` if the code is unknown/expired.
    pub async fn join_realm(&self, code: &str) -> Result<Option<RealmSummary>> {
        self.post_optional_json("/v1/realms/join", &JoinBody { code: code.to_string() }).await
    }

    /// Realms the current user belongs to (owned or joined), newest first.
    pub async fn list_realms(&self) -> Result<Vec<RealmSummary>> {
        self.get_json("/v1/realms/mine").await
    }

    /// A single realm's summary (for the current user).
    pub async fn get_realm(&self, id: &str) -> Result<RealmSummary> {
        self.get_json(&format!("/v1/realms/{id}")).await
    }

    /// The realm's current manifest + version.
    pub async fn realm_manifest(&self, id: &str) -> Result<RealmManifest> {
        self.get_json(&format!("/v1/realms/{id}/manifest")).await
    }

    /// Publish a new manifest (owner/admin only); returns the bumped version.
    pub async fn push_realm_manifest(&self, id: &str, manifest: &RealmManifest) -> Result<i32> {
        let r: VersionResp = self.post_json(&format!("/v1/realms/{id}/manifest"), manifest).await?;
        Ok(r.version)
    }

    /// Member list (only if the current user is a member).
    pub async fn realm_members(&self, id: &str) -> Result<Vec<RealmMember>> {
        self.get_json(&format!("/v1/realms/{id}/members")).await
    }

    /// Owner sets another member's role (`admin`/`member`).
    pub async fn set_member_role(&self, id: &str, uid: &str, role: &str) -> Result<()> {
        self.post_no_content(
            &format!("/v1/realms/{id}/members/{uid}/role"),
            &RoleBody { role: role.to_string() },
        )
        .await
    }

    /// Self-leave, or owner removes a member.
    pub async fn remove_member(&self, id: &str, uid: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/realms/{id}/members/{uid}")).await
    }

    /// Record the manifest version this member has synced to.
    pub async fn mark_realm_synced(&self, id: &str, version: i32) -> Result<()> {
        self.post_no_content(&format!("/v1/realms/{id}/synced"), &SyncedBody { version }).await
    }

    /// Owner disbands the realm.
    pub async fn disband_realm(&self, id: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/realms/{id}")).await
    }
}

/* ---------- syncer ---------- */

/// What a sync to a manifest *would* change, computed without touching disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SyncPlan {
    /// Files in the manifest that are missing locally or fail their hash.
    pub download: Vec<RealmFile>,
    /// Mod filenames present under `mods/` but absent from the manifest.
    pub remove: Vec<String>,
    /// Manifest files with no download url — the member must add them by hand.
    pub manual: Vec<RealmFile>,
    /// Manifest version this plan targets.
    pub version: i32,
}

impl SyncPlan {
    /// True when the instance already matches the manifest (nothing to fetch or
    /// drop). `manual` files don't count — they can't be reconciled automatically.
    pub fn is_up_to_date(&self) -> bool {
        self.download.is_empty() && self.remove.is_empty()
    }
}

/// Outcome of [`apply_sync`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SyncReport {
    pub downloaded: u32,
    pub removed: u32,
    /// Filenames that failed to download (after retries).
    pub failed: Vec<String>,
    /// Files the member must still add by hand.
    pub manual: Vec<RealmFile>,
    /// The manifest version that was applied.
    pub version: i32,
}

/// Does `path` exist and satisfy the manifest file's strongest available hash?
fn file_matches(path: &Path, f: &RealmFile) -> bool {
    if !path.exists() {
        return false;
    }
    if let Some(h) = f.sha512.as_deref().filter(|s| !s.is_empty()) {
        return verify_sha512(path, h);
    }
    if let Some(h) = f.sha1.as_deref().filter(|s| !s.is_empty()) {
        return verify_sha1(path, h);
    }
    // Present, but the manifest gave no hash to check against — accept it.
    true
}

/// Diff an instance against a manifest. Pure (only reads disk); see [`SyncPlan`].
pub fn plan_sync(inst: &Instance, manifest: &RealmManifest) -> SyncPlan {
    let mut download = Vec::new();
    let mut manual = Vec::new();
    // Basenames the manifest expects directly under `mods/` (for stale detection).
    let mut manifest_mods: HashSet<String> = HashSet::new();

    for f in &manifest.files {
        if let Some(name) = f.path.strip_prefix("mods/") {
            manifest_mods.insert(name.to_string());
        }
        match f.url.as_deref() {
            Some(url) if !url.is_empty() => {
                if file_matches(&inst.dir().join(&f.path), f) {
                    continue; // already present + correct
                }
                download.push(f.clone());
            }
            // No url (or empty) ⇒ a custom jar the member must add by hand.
            _ => manual.push(f.clone()),
        }
    }

    // Stale: local mods the manifest no longer references (compare both the raw
    // filename and the `.disabled`-stripped active name).
    let mut remove = Vec::new();
    for m in list_mods(inst) {
        let active = m.file_name.strip_suffix(".disabled").unwrap_or(&m.file_name);
        if !manifest_mods.contains(&m.file_name) && !manifest_mods.contains(active) {
            remove.push(m.file_name);
        }
    }
    remove.sort();

    SyncPlan { download, remove, manual, version: manifest.version }
}

/// Execute a plan: download the missing/changed files and, when `remove_extras`,
/// move the stale mods to the trash. Returns a [`SyncReport`]; downloads that
/// fail after retries are reported, not fatal.
pub async fn apply_sync(
    inst: &Instance,
    dl: &Downloader,
    plan: &SyncPlan,
    remove_extras: bool,
    progress: Option<watch::Sender<Progress>>,
) -> Result<SyncReport> {
    let mut items = Vec::with_capacity(plan.download.len());
    for f in &plan.download {
        let Some(url) = f.url.clone().filter(|u| !u.is_empty()) else { continue };
        let path = inst.dir().join(&f.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Io { path: parent.to_path_buf(), source: e })?;
        }
        items.push(DownloadItem {
            url,
            path,
            sha1: f.sha1.clone(),
            sha512: f.sha512.clone(),
            size: f.size,
            ..Default::default()
        });
    }

    let mut report = SyncReport { version: plan.version, manual: plan.manual.clone(), ..Default::default() };

    if !items.is_empty() {
        let outcome = dl.download_batch(items, progress).await?;
        report.downloaded = outcome.succeeded as u32;
        report.failed = outcome
            .failed
            .iter()
            .map(|(it, _)| {
                it.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            })
            .collect();
    }

    if remove_extras {
        for name in &plan.remove {
            if crate::instance::mods::delete_mod(inst, name).is_ok() {
                report.removed += 1;
            }
        }
    }

    Ok(report)
}

/// Build a manifest from a host's instance: resolve each **enabled** mod jar to a
/// platform download url by its sha1; jars the provider doesn't recognise (or
/// that are blocked from third-party download) become `manual` entries carrying
/// the local hash so members can still verify a hand-placed copy.
pub async fn build_manifest_from_instance(
    inst: &Instance,
    provider: &dyn ResourceProvider,
    mc_version: &str,
    loader: &str,
    loader_version: Option<String>,
) -> Result<RealmManifest> {
    // (filename, size, sha1) for each enabled mod, aligned with `hashes`.
    let mut entries: Vec<(String, u64, String)> = Vec::new();
    let mut hashes: Vec<String> = Vec::new();
    for m in list_mods(inst).into_iter().filter(|m| m.enabled) {
        let path = inst.mods_dir().join(&m.file_name);
        if let Ok(h) = sha1_file(&path) {
            entries.push((m.file_name.clone(), m.size, h.clone()));
            hashes.push(h);
        }
    }

    let resolved = if hashes.is_empty() {
        Vec::new()
    } else {
        provider.resolve_by_hashes(HashAlgo::Sha1, &hashes).await?
    };

    let source = provider_source(provider.id());
    let mut files = Vec::with_capacity(entries.len());
    for (i, (filename, size, sha1)) in entries.into_iter().enumerate() {
        match resolved.get(i).and_then(|r| r.as_ref()) {
            // Resolved to a real, downloadable file.
            Some(rf) if !rf.file.url.is_empty() => {
                let name = if rf.file.filename.is_empty() { filename } else { rf.file.filename.clone() };
                files.push(RealmFile {
                    path: format!("mods/{name}"),
                    sha1: rf.file.sha1.clone().or(Some(sha1)),
                    sha512: rf.file.sha512.clone(),
                    size: rf.file.size.or(Some(size)),
                    url: Some(rf.file.url.clone()),
                    source: Some(source.to_string()),
                });
            }
            // Unresolved, or resolved-but-blocked (empty url) ⇒ manual.
            _ => files.push(RealmFile {
                path: format!("mods/{filename}"),
                sha1: Some(sha1),
                sha512: None,
                size: Some(size),
                url: None,
                source: Some("manual".to_string()),
            }),
        }
    }

    Ok(RealmManifest {
        mc_version: Some(mc_version.to_string()),
        loader: Some(loader.to_string()),
        loader_version,
        files,
        version: 0,
    })
}

fn provider_source(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Modrinth => "modrinth",
        ProviderId::CurseForge => "curseforge",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct TempInst {
        root: PathBuf,
        inst: Instance,
    }

    impl TempInst {
        fn new(tag: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("mc-core-realm-test-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            let inst = Instance::new("1.20.1", root.clone());
            fs::create_dir_all(inst.mods_dir()).unwrap();
            Self { root, inst }
        }

        /// Write a top-level mod jar (any bytes — `list_mods` accepts unparsable
        /// jars and falls back to the filename) and return its sha1.
        fn put_mod(&self, file_name: &str, bytes: &[u8]) -> String {
            let p = self.inst.mods_dir().join(file_name);
            fs::write(&p, bytes).unwrap();
            sha1_file(&p).unwrap()
        }
    }

    impl Drop for TempInst {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn url_file(path: &str, sha1: Option<String>) -> RealmFile {
        RealmFile {
            path: path.into(),
            sha1,
            sha512: None,
            size: None,
            url: Some("https://cdn.example/x.jar".into()),
            source: Some("modrinth".into()),
        }
    }

    #[test]
    fn plan_skips_present_matching_downloads_missing_and_flags_stale_and_manual() {
        let t = TempInst::new("plan");
        // present + correct hash → must be skipped
        let have_sha1 = t.put_mod("present.jar", b"already-here");
        // a local mod the manifest won't mention → stale (remove)
        t.put_mod("extra.jar", b"local-only-util");

        let manifest = RealmManifest {
            mc_version: Some("1.20.1".into()),
            loader: Some("fabric".into()),
            loader_version: None,
            version: 7,
            files: vec![
                url_file("mods/present.jar", Some(have_sha1)), // matches → skip
                url_file("mods/missing.jar", Some("deadbeef".into())), // not on disk → download
                RealmFile {
                    path: "mods/custom.jar".into(),
                    sha1: Some("abc".into()),
                    sha512: None,
                    size: None,
                    url: None, // no url → manual
                    source: Some("manual".into()),
                },
            ],
        };

        let plan = plan_sync(&t.inst, &manifest);
        assert_eq!(plan.version, 7);
        assert_eq!(plan.download.len(), 1, "only the missing url file");
        assert_eq!(plan.download[0].path, "mods/missing.jar");
        assert_eq!(plan.manual.len(), 1);
        assert_eq!(plan.manual[0].path, "mods/custom.jar");
        // `extra.jar` is stale; `present.jar` and the manual `custom.jar` are not.
        assert_eq!(plan.remove, vec!["extra.jar".to_string()]);
        assert!(!plan.is_up_to_date());
    }

    #[test]
    fn plan_is_up_to_date_when_instance_matches_manifest() {
        let t = TempInst::new("uptodate");
        let h = t.put_mod("sodium.jar", b"sodium-bytes");
        let manifest = RealmManifest {
            files: vec![url_file("mods/sodium.jar", Some(h))],
            version: 1,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        assert!(plan.download.is_empty());
        assert!(plan.remove.is_empty());
        assert!(plan.is_up_to_date());
    }

    #[test]
    fn plan_redownloads_on_hash_mismatch() {
        let t = TempInst::new("mismatch");
        t.put_mod("sodium.jar", b"OLD-bytes");
        let manifest = RealmManifest {
            files: vec![url_file("mods/sodium.jar", Some("0000000000000000000000000000000000000000".into()))],
            version: 2,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        assert_eq!(plan.download.len(), 1, "hash mismatch forces re-download");
        // present-but-wrong file is still manifest-referenced → not stale.
        assert!(plan.remove.is_empty());
    }
}
