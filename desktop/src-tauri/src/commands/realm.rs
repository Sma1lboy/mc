use super::*;

// --- private realms (临时领域) + the syncer ----------------------------------
// Thin glue over mc_core::realm: realm CRUD on the held kobeMC ServerClient, and
// the syncer that reconciles an instance's mods/ to a realm manifest. Building a
// manifest from an instance resolves local mod jars to download urls by hash
// (Modrinth provider); the reconcile downloads missing/changed files and can drop
// mods the manifest no longer carries.

use mc_core::realm::{CreateRealmReq, RealmManifest, RealmMember, RealmSummary, SyncPlan, SyncReport};
use mc_core::types::RealmRef;

/// Build the local realm binding (stored on the instance) from a server summary.
/// `loader_version` is filled later from the manifest on "begin" — the summary
/// doesn't carry it.
fn realm_ref(s: &RealmSummary, role: &str) -> RealmRef {
    RealmRef {
        realm_id: s.id.clone(),
        code: Some(s.code.clone()),
        role: role.to_string(),
        name: Some(s.name.clone()),
        mc_version: s.mc_version.clone(),
        loader: s.loader.clone(),
        loader_version: None,
    }
}

/// Build a full snapshot (manifest + optional overrides zip) from a host's
/// instance via the Modrinth provider (always available — no API key needed).
async fn snapshot_of_instance(
    root: &str,
    id: &str,
    mc_version: &str,
    loader: &str,
    loader_version: Option<String>,
) -> CmdResult<(RealmManifest, Option<Vec<u8>>)> {
    let inst = instance_of(root, id);
    let reg = make_registry();
    let provider = provider_or_err(&reg, mc_core::modplatform::ProviderId::Modrinth)?;
    // The frontend's `loader_version` is the instance display id, not a real loader
    // version (see InstanceSummary). For fabric/quilt, derive the actual version from
    // the installed core so members can install the same loader; else members hit
    // `/loader/<mc>/<display-id>/profile/json` → 400. Falls back to None (auto-pick).
    let loader_version = match loader {
        "fabric" | "quilt" => mc_core::instance::resolve_loader_version(&root_paths(root), id, mc_version),
        _ => loader_version,
    };
    mc_core::realm::build_snapshot(&inst, provider.as_ref(), mc_version, loader, loader_version)
        .await
        .map_err(err)
}

/// Download + extract the realm's overrides blob into `inst` when the manifest
/// carries one. Best-effort: a missing/failed blob doesn't fail the whole sync.
/// Extraction runs on a blocking thread (blobs can be large).
async fn apply_overrides_if_any(
    client: &mc_core::server::ServerClient,
    realm_id: &str,
    inst: &Instance,
    manifest: &RealmManifest,
) {
    if manifest.overrides.is_none() {
        return;
    }
    if let Ok(zip) = client.download_overrides(realm_id).await {
        let inst = inst.clone();
        let _ = tokio::task::spawn_blocking(move || mc_core::realm::apply_overrides(&inst, &zip)).await;
    }
}

/// Carry the realm's modpack identity onto the member's instance config (best-effort)
/// so its detail page shows the modpack overview instead of a bare instance. The
/// icon rides the overrides blob, so it's already restored by `apply_overrides_if_any`.
fn apply_manifest_source(inst: &Instance, manifest: &RealmManifest) {
    let Some(src) = manifest.source.as_ref() else { return };
    let Ok(mut config) = inst.load_config() else { return };
    let want = mc_core::instance::config::InstanceSource {
        provider: src.provider.clone(),
        project_id: src.project_id.clone(),
        version_id: src.version_id.clone(),
    };
    if config.source.as_ref() == Some(&want) {
        return;
    }
    config.source = Some(want);
    let _ = inst.save_config(&config);
}

/// Realms the logged-in user belongs to.
#[tauri::command]
#[specta::specta]
pub async fn realm_list(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<RealmSummary>> {
    client.list_realms().await.map_err(err)
}

/// A single realm's summary.
#[tauri::command]
#[specta::specta]
pub async fn realm_get(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
) -> CmdResult<RealmSummary> {
    client.get_realm(&realm_id).await.map_err(err)
}

/// Share an instance as a realm: create it from the instance's current mods, then
/// stamp the realm binding onto that instance (host = owner). Returns the summary.
#[tauri::command]
#[specta::specta]
#[allow(clippy::too_many_arguments)]
pub async fn realm_create(
    client: State<'_, mc_core::server::ServerClient>,
    root: String,
    instance_id: String,
    name: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
    expires_in_secs: Option<i64>,
) -> CmdResult<RealmSummary> {
    let (manifest, overrides) =
        snapshot_of_instance(&root, &instance_id, &mc_version, &loader, loader_version).await?;
    let summary = client
        .create_realm(&CreateRealmReq { name, expires_in_secs, manifest })
        .await
        .map_err(err)?;
    if let Some(zip) = overrides {
        client.upload_overrides(&summary.id, zip).await.map_err(err)?;
    }
    let paths = root_paths(&root);
    let _ = mc_core::instance::lifecycle::set_instance_realm(
        &paths,
        &instance_id,
        Some(realm_ref(&summary, "owner")),
    );
    Ok(summary)
}

/// Join a realm by code and create a **pending** local instance bound to it (no
/// core installed yet — that's "begin"). Returns the new instance id, or `None`
/// if the code is unknown/expired.
#[tauri::command]
#[specta::specta]
pub async fn realm_join(
    client: State<'_, mc_core::server::ServerClient>,
    root: String,
    code: String,
) -> CmdResult<Option<String>> {
    let Some(summary) = client.join_realm(code.trim()).await.map_err(err)? else {
        return Ok(None);
    };
    let paths = root_paths(&root);
    let g = settings_global();
    let id = mc_core::instance::lifecycle::create_realm_shell(
        &paths,
        &summary.name,
        realm_ref(&summary, &summary.role),
        g.default_memory_mb,
        g.java_path.clone(),
    )
    .map_err(err)?;
    Ok(Some(id))
}

/// "Begin": for a freshly-joined (pending) instance, install the core (version +
/// loader from the manifest) then download the realm's mods. Idempotent on the
/// core. Progress streams over `realm://sync-progress`.
#[tauri::command]
#[specta::specta]
pub async fn realm_begin(
    app: AppHandle,
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<SyncReport> {
    let paths = root_paths(&root);
    let inst = instance_of(&root, &instance_id);
    let manifest = client.realm_manifest(&realm_id).await.map_err(err)?;
    let dl = make_downloader()?;
    let tx = progress_channel(app, "realm://sync-progress", "准备");

    // 1) install the core (version + loader) — idempotent.
    let mc_version = manifest.mc_version.clone().unwrap_or_default();
    let loader_opt = match parse_loader_kind(manifest.loader.as_deref().unwrap_or("")) {
        None | Some(mc_core::types::LoaderKind::Vanilla) => None,
        Some(kind) => {
            // Defensive: older manifests stored the instance display id (which contains
            // spaces) as the loader version — not a real loader version. Blank it so the
            // installer auto-picks the latest loader compatible with `mc_version`.
            let lv = manifest.loader_version.clone().unwrap_or_default();
            let lv = if lv.contains(' ') { String::new() } else { lv };
            Some((kind, lv))
        }
    };
    mc_core::instance::lifecycle::materialize_core(&dl, &paths, &instance_id, &mc_version, loader_opt, Some(tx.clone()))
        .await
        .map_err(err)?;

    // 2) download the realm's mods.
    let plan = mc_core::realm::plan_sync(&inst, &manifest);
    let report = mc_core::realm::apply_sync(&inst, &dl, &plan, false, Some(tx)).await.map_err(err)?;

    // 3) extract the overrides blob (config/scripts/icon/non-CDN content), if any.
    apply_overrides_if_any(&client, &realm_id, &inst, &manifest).await;
    // 4) keep the modpack source so this member's instance detail shows the overview.
    apply_manifest_source(&inst, &manifest);

    let _ = client.mark_realm_synced(&realm_id, report.version).await;
    Ok(report)
}

/// EasyTier lobby credentials for a realm (members only) — network name/secret +
/// external nodes (P2P public + optional our hosted relay). P1: fetch only.
#[tauri::command]
#[specta::specta]
pub async fn realm_lobby(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
) -> CmdResult<mc_core::lobby::LobbyCreds> {
    client.realm_lobby(&realm_id).await.map_err(err)
}

/// 联机大厅 P3 —— host 发布我可达的地址(`<虚拟IP>:<端口>`),成员据此一键加入。
/// 边开世界边每 ~30s 调一次作心跳(server 端 90s 过期)。
#[tauri::command]
#[specta::specta]
pub async fn realm_set_host(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    address: String,
) -> CmdResult<()> {
    client.realm_set_host(&realm_id, &address).await.map_err(err)
}

/// 联机大厅 P3 —— 查领域当前(新鲜的)host:有人在主持则返回 `address` + `host_username`,
/// 否则两者皆 `None`。成员轮询它来决定能否「加入游戏」。
#[tauri::command]
#[specta::specta]
pub async fn realm_get_host(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
) -> CmdResult<mc_core::realm::RealmHost> {
    client.realm_get_host(&realm_id).await.map_err(err)
}

/// 联机大厅 P3 —— 停止主持(清掉我的 host 记录)。非 host 调用是无害空操作。
#[tauri::command]
#[specta::specta]
pub async fn realm_clear_host(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
) -> CmdResult<()> {
    client.realm_clear_host(&realm_id).await.map_err(err)
}

/// 联机大厅 P3 —— 探测本机 Minecraft 是否「对局域网开放」:加入 MC 局域网发现组播监听
/// ~3s,读到端口则返回。未开 / 探测失败 → `None`(绝不 panic / 阻塞超过 ~3s)。
#[tauri::command]
#[specta::specta]
pub async fn detect_lan_world() -> CmdResult<Option<u16>> {
    Ok(mc_core::lan_world::detect_lan_port(std::time::Duration::from_secs(3)).await)
}

/// Member list (with synced-version progress).
#[tauri::command]
#[specta::specta]
pub async fn realm_members(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
) -> CmdResult<Vec<RealmMember>> {
    client.realm_members(&realm_id).await.map_err(err)
}

/// Owner/admin republishes the manifest from an instance; returns new version.
#[tauri::command]
#[specta::specta]
pub async fn realm_push_manifest(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
    mc_version: String,
    loader: String,
    loader_version: Option<String>,
) -> CmdResult<i32> {
    let (manifest, overrides) =
        snapshot_of_instance(&root, &instance_id, &mc_version, &loader, loader_version).await?;
    let version = client.push_realm_manifest(&realm_id, &manifest).await.map_err(err)?;
    if let Some(zip) = overrides {
        client.upload_overrides(&realm_id, zip).await.map_err(err)?;
    }
    Ok(version)
}

/// Dry-run: what syncing `instance_id` to the realm's manifest would change.
#[tauri::command]
#[specta::specta]
pub async fn realm_plan_sync(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<SyncPlan> {
    let manifest = client.realm_manifest(&realm_id).await.map_err(err)?;
    Ok(mc_core::realm::plan_sync(&instance_of(&root, &instance_id), &manifest))
}

/// Reconcile `instance_id` to the realm manifest: download missing/changed mods,
/// optionally drop the ones the manifest no longer carries, then report progress
/// to the server. Progress streams over a dedicated `realm://sync-progress` event
/// (kept off `install://progress` so it can't collide with a concurrent install).
#[tauri::command]
#[specta::specta]
pub async fn realm_sync(
    app: AppHandle,
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
    remove_extras: bool,
) -> CmdResult<SyncReport> {
    let inst = instance_of(&root, &instance_id);
    let manifest = client.realm_manifest(&realm_id).await.map_err(err)?;
    let plan = mc_core::realm::plan_sync(&inst, &manifest);

    let dl = make_downloader()?;
    let tx = progress_channel(app, "realm://sync-progress", "同步领域");
    let report = mc_core::realm::apply_sync(&inst, &dl, &plan, remove_extras, Some(tx)).await.map_err(err)?;

    // Extract the overrides blob (config/scripts/icon/non-CDN content), if any.
    apply_overrides_if_any(&client, &realm_id, &inst, &manifest).await;
    // Keep the modpack source so this member's instance detail shows the overview.
    apply_manifest_source(&inst, &manifest);

    // Best-effort: record how far this member has synced (don't fail the sync).
    let _ = client.mark_realm_synced(&realm_id, report.version).await;
    Ok(report)
}

/// Owner sets a member's role (`admin`/`member`).
#[tauri::command]
#[specta::specta]
pub async fn realm_set_role(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
    role: String,
) -> CmdResult<()> {
    client.set_member_role(&realm_id, &user_id, &role).await.map_err(err)
}

/// Owner removes another member (their own client clears its binding locally).
#[tauri::command]
#[specta::specta]
pub async fn realm_remove_member(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
) -> CmdResult<()> {
    client.remove_member(&realm_id, &user_id).await.map_err(err)
}

/// Owner/admin invites an accepted friend straight into the realm (no join code).
#[tauri::command]
#[specta::specta]
pub async fn realm_invite(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
) -> CmdResult<()> {
    client.realm_invite(&realm_id, &user_id).await.map_err(err)
}

/// Self-leave a realm and unbind it from the local instance (the instance stays;
/// if it was never synced it's just an empty shell that drops out of the list).
#[tauri::command]
#[specta::specta]
pub async fn realm_leave(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    user_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<()> {
    client.remove_member(&realm_id, &user_id).await.map_err(err)?;
    let paths = root_paths(&root);
    let _ = mc_core::instance::lifecycle::set_instance_realm(&paths, &instance_id, None);
    Ok(())
}

/// Owner disbands the realm and unbinds it from the local instance.
#[tauri::command]
#[specta::specta]
pub async fn realm_disband(
    client: State<'_, mc_core::server::ServerClient>,
    realm_id: String,
    root: String,
    instance_id: String,
) -> CmdResult<()> {
    client.disband_realm(&realm_id).await.map_err(err)?;
    let paths = root_paths(&root);
    let _ = mc_core::instance::lifecycle::set_instance_realm(&paths, &instance_id, None);
    Ok(())
}

