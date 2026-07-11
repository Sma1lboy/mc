use super::*;

fn ise(_: anyhow::Error) -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

/// `POST /v1/realms`
pub async fn create(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateRealmReq>,
) -> Result<Json<RealmSummary>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    s.realms.create(&user, &req).await.map(Json).map_err(ise)
}

/// `POST /v1/realms/join`
pub async fn join(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<JoinReq>,
) -> Result<Json<RealmSummary>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    s.realms.join(&user, &req.code).await.map_err(ise)?.map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// `GET /v1/realms/mine`
pub async fn list_mine(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<RealmSummary>>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    s.realms.list_mine(&user).await.map(Json).map_err(ise)
}

/// `GET /v1/realms/{id}`
pub async fn get(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RealmSummary>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    s.realms.summary_for(&id, &user).await.map_err(ise)?.map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// `GET /v1/realms/{id}/manifest`
pub async fn get_manifest(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RealmManifest>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    s.realms.manifest(&id, &user).await.map_err(ise)?.map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// `POST /v1/realms/{id}/manifest` — owner/admin only.
pub async fn push_manifest(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(manifest): Json<RealmManifest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    match s.realms.push_manifest(&id, &user, &manifest).await.map_err(ise)? {
        Some(version) => Ok(Json(serde_json::json!({ "version": version }))),
        None => Err(StatusCode::FORBIDDEN),
    }
}

/// `GET /v1/realms/{id}/members`
pub async fn members(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Vec<MemberInfo>>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    s.realms.members(&id, &user).await.map_err(ise)?.map(Json).ok_or(StatusCode::FORBIDDEN)
}

/// `POST /v1/realms/{id}/invite` — owner/admin invites an accepted friend
/// straight into the realm (no join code). `403` if the actor isn't owner/admin
/// or the target isn't an accepted friend; `404` if the target user is missing.
pub async fn invite(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<crate::friend::UserIdReq>,
) -> Result<StatusCode, StatusCode> {
    let actor = require_user(&s.pool, &headers).await?;
    if !s.realms.can_push(&id, &actor).await.map_err(ise)? {
        return Err(StatusCode::FORBIDDEN);
    }
    if !s.friends.are_friends(&actor, &req.user_id).await.map_err(ise)? {
        return Err(StatusCode::FORBIDDEN);
    }
    s.realms.add_member(&id, &req.user_id).await.map_err(|_| StatusCode::NOT_FOUND)?;
    // Tell the invitee they were added to the realm (best-effort).
    let _ = s.notifications.create(&req.user_id, "realm_invite", Some(&actor), Some(&id)).await;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/realms/{id}/members/{uid}/role` — owner only.
pub async fn set_role(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((id, uid)): Path<(String, String)>,
    Json(req): Json<RoleReq>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    if req.role != "admin" && req.role != "member" {
        return Err(StatusCode::BAD_REQUEST);
    }
    let ok = s.realms.set_role(&id, &user, &uid, &req.role).await.map_err(ise)?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::FORBIDDEN) }
}

/// `DELETE /v1/realms/{id}/members/{uid}` — self-leave or owner-remove.
pub async fn remove_member(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path((id, uid)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    let ok = s.realms.leave_or_remove(&id, &user, &uid).await.map_err(ise)?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::FORBIDDEN) }
}

/// `POST /v1/realms/{id}/synced` — member reports its synced version.
pub async fn mark_synced(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<SyncedReq>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    s.realms.mark_synced(&id, &user, req.version).await.map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/realms/{id}` — owner disbands.
pub async fn disband(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    let ok = s.realms.delete(&id, &user).await.map_err(ise)?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::FORBIDDEN) }
}

/// `POST /v1/realms/{id}/overrides` — owner/admin uploads the overrides zip blob
/// (raw body). Stored on the server's blob volume keyed by realm id.
pub async fn upload_overrides(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    if !s.realms.can_push(&id, &user).await.map_err(ise)? {
        return Err(StatusCode::FORBIDDEN);
    }
    let _ = std::fs::create_dir_all(&s.blob_dir);
    let path = s.blob_dir.join(format!("{id}.zip"));
    std::fs::write(&path, &body).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/realms/{id}/overrides` — member downloads the overrides zip blob.
pub async fn get_overrides(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<(axum::http::HeaderMap, Vec<u8>), StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    if !s.realms.is_member(&id, &user).await.map_err(ise)? {
        return Err(StatusCode::FORBIDDEN);
    }
    let path = s.blob_dir.join(format!("{id}.zip"));
    let bytes = std::fs::read(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    let mut h = axum::http::HeaderMap::new();
    h.insert(axum::http::header::CONTENT_TYPE, axum::http::HeaderValue::from_static("application/zip"));
    Ok((h, bytes))
}

/* ---------- lobby host (联机大厅 P3): who's hosting the LAN-opened world ---------- */

#[derive(Deserialize)]
pub struct SetHostReq {
    pub address: String,
}

/// Who is currently hosting a realm's LAN-opened world (fresh only).
#[derive(Serialize)]
pub struct RealmHost {
    pub address: Option<String>,
    pub host_username: Option<String>,
}

/// `POST /v1/realms/{id}/host` — publish/heartbeat my reachable address
/// (`<virtual_ip>:<lan_port>`). Membership-gated. Call ~every 30s while hosting.
pub async fn set_host(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<SetHostReq>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    if s.realms.summary_for(&id, &user).await.map_err(ise)?.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    s.realms.set_host(&id, &user, &req.address).await.map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/realms/{id}/host` — the realm's current (fresh) host, membership-gated.
/// `{ address: null, host_username: null }` when nobody is hosting.
pub async fn get_host(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<RealmHost>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    if s.realms.summary_for(&id, &user).await.map_err(ise)?.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    let (address, host_username) = s.realms.get_host(&id).await.map_err(ise)?;
    Ok(Json(RealmHost { address, host_username }))
}

/// `DELETE /v1/realms/{id}/host` — stop hosting (clears my host record).
/// Membership-gated; only clears if I'm the current host (else a harmless no-op).
pub async fn clear_host(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    if s.realms.summary_for(&id, &user).await.map_err(ise)?.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    s.realms.clear_host(&id, &user).await.map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

/* ---------- lobby (联机大厅): EasyTier room credentials ---------- */

/// One EasyTier external/relay node the client can use for rendezvous (+ relay).
/// `kind`: `"p2p"` = a public shared node (direct after hole-punch, no cost to us);
/// `"hosted"` = our own relay (our "host point", used when punch-through fails).
#[derive(Serialize, Clone)]
pub struct LobbyNode {
    pub kind: String,
    pub name: String,
    pub addr: String,
}

/// EasyTier room credentials for a realm. All members of a realm get the SAME
/// `network_name` + `network_secret`, so they land on one virtual LAN; the
/// secret is derived from the server's AUTH_SECRET (never guessable by non-members,
/// and the endpoint is membership-gated). `nodes` lists the external nodes to try.
#[derive(Serialize, Clone)]
pub struct LobbyCreds {
    pub network_name: String,
    pub network_secret: String,
    pub nodes: Vec<LobbyNode>,
}

/// Derive a stable, unguessable network secret for a realm from the server secret.
fn lobby_secret(realm_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let server_secret =
        std::env::var("AUTH_SECRET").unwrap_or_else(|_| "dev-only-insecure-secret-change-me-0123456789".to_string());
    let mut h = Sha256::new();
    h.update(server_secret.as_bytes());
    h.update(b":lobby:");
    h.update(realm_id.as_bytes());
    let digest = h.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// `GET /v1/realms/{id}/lobby` — EasyTier room credentials for a member.
/// Members only (403 if not a member). The P2P node + (optional) our hosted relay
/// come from env: `MC_LOBBY_P2P_NODE` (default EasyTier public), `MC_LOBBY_RELAY`.
pub async fn lobby(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<LobbyCreds>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    // Membership gate: summary_for returns None when the user isn't a member.
    if s.realms.summary_for(&id, &user).await.map_err(ise)?.is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut nodes = Vec::new();
    let p2p = std::env::var("MC_LOBBY_P2P_NODE")
        .unwrap_or_else(|_| "tcp://public.easytier.cn:11010".to_string());
    if !p2p.trim().is_empty() {
        nodes.push(LobbyNode { kind: "p2p".into(), name: "EasyTier Public".into(), addr: p2p });
    }
    if let Ok(relay) = std::env::var("MC_LOBBY_RELAY") {
        if !relay.trim().is_empty() {
            nodes.push(LobbyNode { kind: "hosted".into(), name: "kobeMC Relay".into(), addr: relay });
        }
    }
    Ok(Json(LobbyCreds {
        network_name: format!("kobe-{id}"),
        network_secret: lobby_secret(&id),
        nodes,
    }))
}
