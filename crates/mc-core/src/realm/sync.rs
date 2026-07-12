use super::*;

/// Per-instance sync ledger (`realm-sync.json` in the instance dir): the paths
/// this syncer itself reconciled to the manifest. Only ledger-tracked paths are
/// ever removal candidates when a later manifest drops them — files the member
/// added by hand were never in the ledger, so the syncer never deletes them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    /// Manifest version last applied.
    pub version: i32,
    /// Instance-relative paths the syncer installed/verified as manifest content.
    #[serde(default)]
    pub installed: Vec<String>,
}

pub(crate) const SYNC_STATE_FILE: &str = "realm-sync.json";

/// Load the instance's sync ledger; missing/corrupt file ⇒ empty (first sync).
pub fn load_sync_state(inst: &Instance) -> SyncState {
    std::fs::read(inst.dir().join(SYNC_STATE_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_sync_state(inst: &Instance, state: &SyncState) -> Result<()> {
    let path = inst.dir().join(SYNC_STATE_FILE);
    let data = serde_json::to_vec_pretty(state)
        .map_err(|e| CoreError::Parse { what: "realm-sync.json".into(), source: e })?;
    std::fs::write(&path, data).map_err(|e| CoreError::Io { path, source: e })
}

/// What a sync to a manifest *would* change, computed without touching disk.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct SyncPlan {
    /// Files in the manifest that are missing locally or fail their hash.
    pub download: Vec<RealmFile>,
    /// Previously syncer-installed paths (per the [`SyncState`] ledger) that the
    /// current manifest dropped and that still exist locally. Never includes
    /// files the member added themselves.
    pub remove: Vec<String>,
    /// Every safe, url-carrying manifest path — what the ledger will record as
    /// syncer-managed once the plan is applied.
    pub managed: Vec<String>,
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
    let mut managed = Vec::new();
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
                managed.push(f.path.clone());
                if file_matches(&target, f) {
                    continue; // already present + correct
                }
                download.push(f.clone());
            }
            // No url (or empty) ⇒ a custom file the member must add by hand.
            _ => manual.push(f.clone()),
        }
    }

    // Stale: paths the syncer itself installed (per the ledger) that the current
    // manifest dropped and that are still on disk. Files the member added by hand
    // were never in the ledger, so they are never removal candidates. A member who
    // merely disabled a synced mod (`x.jar` → `x.jar.disabled`) still gets the
    // `.disabled` twin cleaned up when the manifest drops the mod.
    let mut remove = Vec::new();
    for path in &load_sync_state(inst).installed {
        if manifest_paths.contains(path) {
            continue;
        }
        let Some(abs) = safe_target(inst, path) else { continue };
        if abs.exists() {
            remove.push(path.clone());
        } else {
            let disabled = format!("{path}.disabled");
            if safe_target(inst, &disabled).is_some_and(|p| p.exists()) {
                remove.push(disabled);
            }
        }
    }
    remove.sort();
    remove.dedup();

    SyncPlan { download, remove, manual, managed, version: manifest.version }
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

    // Absolute targets that failed to download — kept out of the ledger so the
    // next plan retries them instead of treating them as installed.
    let mut failed_abs: HashSet<std::path::PathBuf> = HashSet::new();
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
        failed_abs = outcome.failed.into_iter().map(|(it, _)| it.path).collect();
    }

    if remove_extras {
        for rel in &plan.remove {
            let Some(p) = safe_target(inst, rel) else { continue };
            if p.exists() && (trash::delete(&p).is_ok() || std::fs::remove_file(&p).is_ok()) {
                report.removed += 1;
            }
        }
    }

    // Ledger: everything the manifest manages and we now hold (pre-existing match
    // or fresh download), plus dropped-but-still-present paths (removal declined
    // or failed) so future plans keep offering to clean them up.
    let mut installed: Vec<String> = plan
        .managed
        .iter()
        .filter(|p| safe_target(inst, p).is_some_and(|abs| !failed_abs.contains(&abs)))
        .cloned()
        .collect();
    installed.extend(
        plan.remove.iter().filter(|p| safe_target(inst, p).is_some_and(|abs| abs.exists())).cloned(),
    );
    if let Err(e) = save_sync_state(inst, &SyncState { version: plan.version, installed }) {
        tracing::warn!(target: "mc_core::realm", err = %e, "写入领域同步账本失败");
    }

    Ok(report)
}

/// Build a full snapshot of a host's instance: a manifest (CDN-resolved files
/// across [`MANAGED_DIRS`]) **plus** an overrides zip carrying every other host
/// customisation — config/scripts, options.txt, servers.dat, icon.png, any
/// modpack-shipped dir, and any managed-dir file the provider doesn't recognise —
/// excluding personal data (saves/screenshots) and launcher internals (core
/// jar/json, our instance.json). The zip is uploaded to the realm; the manifest
/// only holds its [`RealmOverrides`] descriptor. Returns `(manifest, Some(zip))`,
/// or a `None` zip when there are no non-CDN files.
pub async fn build_snapshot(
    inst: &Instance,
    provider: &dyn ResourceProvider,
    mc_version: &str,
    loader: &str,
    loader_version: Option<String>,
) -> Result<(RealmManifest, Option<Vec<u8>>)> {
    // (path-relative-to-instance, size, sha1) for every managed-dir file, aligned with `hashes`.
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
    let mut override_paths: Vec<String> = Vec::new();
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
            // Unresolved / blocked ⇒ deliver verbatim via the overrides blob.
            _ => override_paths.push(path),
        }
    }

    // Everything else the host customised rides the overrides blob — not just
    // config/scripts but options.txt, servers.dat, icon.png, OptiFine/shader option
    // files, and any modpack-shipped dir. Walk the whole instance tree, excluding:
    //   - the CDN-managed dirs (handled above: resolved → url, unresolved → blob);
    //   - personal data (saves / screenshots) the host shouldn't push onto members;
    //   - launcher internals (the core jar/json, natives dir, our instance.json).
    // (walk_game_root already hard-ignores logs / crash-reports / loader caches.)
    let id = inst.version_id();
    let ignores: Vec<String> = MANAGED_DIRS
        .iter()
        .map(|s| (*s).to_string())
        .chain(["saves", "screenshots", "natives"].into_iter().map(str::to_string))
        .chain([
            "instance.json".to_string(),
            SYNC_STATE_FILE.to_string(),
            format!("{id}.jar"),
            format!("{id}.json"),
        ])
        .collect();
    for wf in walk_game_root(&inst.dir(), &ignores).unwrap_or_default() {
        override_paths.push(wf.rel);
    }

    let zip = build_overrides_zip(inst, &override_paths)?;
    let overrides = zip
        .as_ref()
        .map(|b| RealmOverrides { sha1: crate::download::checksum::sha1_bytes(b), size: b.len() as u64 });

    // Carry the modpack identity (if this instance was installed from one) so
    // members keep the source on their synced instance → modpack detail works.
    let source = inst.load_config().ok().and_then(|c| c.source).map(|s| RealmSource {
        provider: s.provider,
        project_id: s.project_id,
        version_id: s.version_id,
    });

    Ok((
        RealmManifest {
            mc_version: Some(mc_version.to_string()),
            loader: Some(loader.to_string()),
            loader_version,
            files,
            overrides,
            source,
            version: 0,
        },
        zip,
    ))
}

/// Zip the given instance-relative paths into an in-memory blob (zip-slip guarded).
/// `None` when nothing readable was added.
pub(crate) fn build_overrides_zip(inst: &Instance, rel_paths: &[String]) -> Result<Option<Vec<u8>>> {
    use std::io::Write;
    if rel_paths.is_empty() {
        return Ok(None);
    }
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut wrote = false;
    for rel in rel_paths {
        let Some(abs) = crate::fs::safe_join(&inst.dir(), rel) else { continue };
        let Ok(data) = std::fs::read(&abs) else { continue };
        writer.start_file(rel.as_str(), options).map_err(|e| CoreError::Zip(e.to_string()))?;
        writer.write_all(&data).map_err(|e| CoreError::Zip(e.to_string()))?;
        wrote = true;
    }
    let cursor = writer.finish().map_err(|e| CoreError::Zip(e.to_string()))?;
    Ok(if wrote { Some(cursor.into_inner()) } else { None })
}

/// Extract an overrides zip into the instance (zip-slip guarded via `safe_join`).
/// Returns how many files were written.
pub fn apply_overrides(inst: &Instance, zip_bytes: &[u8]) -> Result<u32> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))
        .map_err(|e| CoreError::Zip(e.to_string()))?;
    let mut n = 0u32;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| CoreError::Zip(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        let Some(dest) = crate::fs::safe_join(&inst.dir(), &name) else {
            tracing::warn!(target: "mc_core::realm", path = %name, "跳过越界的 overrides 条目");
            continue;
        };
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Io { path: parent.to_path_buf(), source: e })?;
        }
        let mut out = std::fs::File::create(&dest)
            .map_err(|e| CoreError::Io { path: dest.clone(), source: e })?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|e| CoreError::Io { path: dest.clone(), source: e })?;
        n += 1;
    }
    Ok(n)
}

fn provider_source(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Modrinth => "modrinth",
        ProviderId::CurseForge => "curseforge",
    }
}
