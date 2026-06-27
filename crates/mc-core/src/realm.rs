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
    /// Paths (relative to the instance dir) under the managed dirs that are
    /// present locally but absent from the manifest.
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

/// Resolve a manifest file's **server-controlled** `path` to a safe absolute
/// target under one of the [`MANAGED_DIRS`], or `None` if it escapes — absolute,
/// contains `..`, or is declared outside a managed dir. The manifest is
/// owner/admin-controlled, so this is the trust boundary: without it a crafted
/// manifest could make the downloader write anywhere (the same defense the mrpack
/// importer gets from [`crate::fs::safe_join`]).
fn safe_target(inst: &Instance, path: &str) -> Option<std::path::PathBuf> {
    for d in MANAGED_DIRS {
        if let Some(rel) = path.strip_prefix(&format!("{d}/")) {
            return crate::fs::safe_join(&inst.dir().join(d), rel);
        }
    }
    None
}

/// List every file under one of the instance's managed dirs, as paths relative to
/// that dir (`/`-separated). Missing dir → empty; hard-ignored junk skipped.
fn managed_dir_files(inst: &Instance, dir: &str) -> Vec<String> {
    let base = inst.dir().join(dir);
    walk_game_root(&base, &[]).map(|fs| fs.into_iter().map(|f| f.rel).collect()).unwrap_or_default()
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
    // Full relative paths the manifest expects (for stale detection).
    let mut manifest_paths: HashSet<String> = HashSet::new();

    for f in &manifest.files {
        // The path is owner/admin-controlled; reject anything that escapes the
        // managed dirs (absolute / `..` / other dir) so a crafted manifest can't
        // probe or target files outside the instance.
        let Some(target) = safe_target(inst, &f.path) else {
            tracing::warn!(target: "mc_core::realm", path = %f.path, "拒绝越界的领域清单路径");
            continue;
        };
        manifest_paths.insert(f.path.clone());
        match f.url.as_deref() {
            Some(url) if !url.is_empty() => {
                if file_matches(&target, f) {
                    continue; // already present + correct
                }
                download.push(f.clone());
            }
            // No url (or empty) ⇒ a custom file the member must add by hand.
            _ => manual.push(f.clone()),
        }
    }

    // Stale: files under the managed dirs the manifest no longer references
    // (compare both the raw path and the `.disabled`-stripped active path).
    let mut remove = Vec::new();
    for d in MANAGED_DIRS {
        for rel in managed_dir_files(inst, d) {
            let full = format!("{d}/{rel}");
            let active = full.strip_suffix(".disabled").unwrap_or(&full);
            if !manifest_paths.contains(&full) && !manifest_paths.contains(active) {
                remove.push(full);
            }
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
        // Defense in depth — the plan crossed an IPC boundary; re-validate the path.
        let Some(path) = safe_target(inst, &f.path) else {
            tracing::warn!(target: "mc_core::realm", path = %f.path, "跳过越界的下载路径");
            continue;
        };
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
        for rel in &plan.remove {
            let Some(p) = safe_target(inst, rel) else { continue };
            if p.exists() && (trash::delete(&p).is_ok() || std::fs::remove_file(&p).is_ok()) {
                report.removed += 1;
            }
        }
    }

    Ok(report)
}

/// Build a manifest from a host's instance: walk every file under the managed
/// content dirs ([`MANAGED_DIRS`]) and resolve each to a platform download url by
/// its sha1; files the provider doesn't recognise (or that are blocked from
/// third-party download) become `manual` entries carrying the local hash so
/// members can still verify a hand-placed copy.
pub async fn build_manifest_from_instance(
    inst: &Instance,
    provider: &dyn ResourceProvider,
    mc_version: &str,
    loader: &str,
    loader_version: Option<String>,
) -> Result<RealmManifest> {
    // (path-relative-to-instance, size, sha1) for every file under the managed
    // dirs (skipping disabled), aligned with `hashes`.
    let mut entries: Vec<(String, u64, String)> = Vec::new();
    let mut hashes: Vec<String> = Vec::new();
    for d in MANAGED_DIRS {
        for wf in walk_game_root(&inst.dir().join(d), &[]).unwrap_or_default() {
            if wf.rel.ends_with(".disabled") {
                continue; // disabled content isn't part of the shared set
            }
            if let Ok(h) = wf.hash(HashAlgo::Sha1) {
                entries.push((format!("{d}/{}", wf.rel), wf.size, h.clone()));
                hashes.push(h);
            }
        }
    }

    let resolved = if hashes.is_empty() {
        Vec::new()
    } else {
        provider.resolve_by_hashes(HashAlgo::Sha1, &hashes).await?
    };

    let source = provider_source(provider.id());
    let mut files = Vec::with_capacity(entries.len());
    for (i, (path, size, sha1)) in entries.into_iter().enumerate() {
        match resolved.get(i).and_then(|r| r.as_ref()) {
            // Resolved to a real, downloadable file — keep the original install path.
            Some(rf) if !rf.file.url.is_empty() => {
                files.push(RealmFile {
                    path,
                    sha1: rf.file.sha1.clone().or(Some(sha1)),
                    sha512: rf.file.sha512.clone(),
                    size: rf.file.size.or(Some(size)),
                    url: Some(rf.file.url.clone()),
                    source: Some(source.to_string()),
                });
            }
            // Unresolved, or resolved-but-blocked (empty url) ⇒ manual (Phase 2:
            // these ride the overrides blob instead).
            _ => files.push(RealmFile {
                path,
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
    use crate::download::checksum::sha1_file;
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

        /// Write a top-level mod jar and return its sha1.
        fn put_mod(&self, file_name: &str, bytes: &[u8]) -> String {
            self.put_file(&format!("mods/{file_name}"), bytes)
        }

        /// Write a file at `rel` (relative to the instance dir) and return its sha1.
        fn put_file(&self, rel: &str, bytes: &[u8]) -> String {
            let p = self.inst.dir().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
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
        assert_eq!(plan.remove, vec!["mods/extra.jar".to_string()]);
        assert!(!plan.is_up_to_date());
    }

    #[test]
    fn plan_covers_resourcepacks_and_shaders_not_just_mods() {
        let t = TempInst::new("multidir");
        // a present, matching resourcepack → skipped; a missing shader → download;
        // a stale local resourcepack → remove.
        let rp = t.put_file("resourcepacks/faithful.zip", b"rp-bytes");
        t.put_file("resourcepacks/stale-rp.zip", b"old-rp");
        let manifest = RealmManifest {
            files: vec![
                url_file("resourcepacks/faithful.zip", Some(rp)),
                url_file("shaderpacks/complementary.zip", Some("deadbeef".into())),
            ],
            version: 1,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        assert_eq!(plan.download.len(), 1);
        assert_eq!(plan.download[0].path, "shaderpacks/complementary.zip");
        assert_eq!(plan.remove, vec!["resourcepacks/stale-rp.zip".to_string()]);
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
    fn plan_rejects_path_traversal_and_absolute_paths() {
        let t = TempInst::new("traversal");
        let manifest = RealmManifest {
            files: vec![
                url_file("../../evil.sh", Some("x".into())),        // parent escape
                url_file("/etc/cron.d/evil", Some("x".into())),     // absolute
                url_file("mods/../../escape.jar", Some("x".into())), // escape via ..
                url_file("config/evil.toml", Some("x".into())),     // outside mods/
                url_file("mods/ok.jar", Some("deadbeef".into())),   // the only legit one
            ],
            version: 1,
            ..Default::default()
        };
        let plan = plan_sync(&t.inst, &manifest);
        // Only the legit, missing mods/ok.jar is scheduled; every escaping path dropped.
        assert_eq!(plan.download.len(), 1);
        assert_eq!(plan.download[0].path, "mods/ok.jar");
        assert!(plan.manual.is_empty());
        assert!(plan.remove.is_empty());
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
