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

// Columns selected for a RealmSummary (realm joined to the asking member).
type SummaryRow = (String, String, String, String, Option<String>, Option<String>, i32, String, Option<String>);
// (user_id, username, role, synced_version, joined_at) for the members query.
type MemberRow = (String, Option<String>, String, i32, Option<String>);
const SUMMARY_COLS: &str = "r.id, r.code, r.name, r.owner_id, r.mc_version, r.loader, r.manifest_version, m.role, r.expires_at::text";

fn row_to_summary(r: SummaryRow) -> RealmSummary {
    RealmSummary {
        id: r.0,
        code: r.1,
        name: r.2,
        owner_id: r.3,
        mc_version: r.4,
        loader: r.5,
        manifest_version: r.6,
        role: r.7,
        expires_at: r.8,
    }
}

#[derive(Clone)]
pub struct RealmStore {
    pool: PgPool,
}

impl RealmStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a realm owned by `owner_id`; mints id + code, seeds manifest at
    /// version 1, and enrolls the owner as the `owner` member.
    pub async fn create(&self, owner_id: &str, req: &CreateRealmReq) -> anyhow::Result<RealmSummary> {
        let manifest_json = serde_json::to_string(&req.manifest)?;
        let expires = req.expires_in_secs.unwrap_or(0);
        let (id,): (String,) = sqlx::query_as(
            "INSERT INTO realms (id, code, name, owner_id, mc_version, loader, manifest, manifest_version, expires_at) \
             VALUES (gen_random_uuid()::text, upper(substr(md5(gen_random_uuid()::text), 1, 6)), \
                     $1, $2, $3, $4, $5::jsonb, 1, \
                     CASE WHEN $6 > 0 THEN NOW() + ($6 * INTERVAL '1 second') ELSE NULL END) \
             RETURNING id",
        )
        .bind(&req.name)
        .bind(owner_id)
        .bind(&req.manifest.mc_version)
        .bind(&req.manifest.loader)
        .bind(&manifest_json)
        .bind(expires)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query("INSERT INTO realm_members (realm_id, user_id, role) VALUES ($1, $2, 'owner')")
            .bind(&id)
            .bind(owner_id)
            .execute(&self.pool)
            .await?;

        self.summary_for(&id, owner_id).await?.ok_or_else(|| anyhow::anyhow!("realm vanished after create"))
    }

    /// Join by code (if not expired); idempotent. Returns the summary, or `None`
    /// if the code is unknown/expired.
    pub async fn join(&self, user_id: &str, code: &str) -> anyhow::Result<Option<RealmSummary>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM realms WHERE code = $1 AND (expires_at IS NULL OR expires_at > NOW())",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?;
        let Some((id,)) = row else { return Ok(None) };

        sqlx::query(
            "INSERT INTO realm_members (realm_id, user_id, role) VALUES ($1, $2, 'member') \
             ON CONFLICT (realm_id, user_id) DO NOTHING",
        )
        .bind(&id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        self.summary_for(&id, user_id).await
    }

    /// Insert `target_id` as a plain `member` (idempotent). The caller must
    /// authorize first. Errors (FK violation) ⇒ the target user doesn't exist.
    pub async fn add_member(&self, realm_id: &str, target_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO realm_members (realm_id, user_id, role) VALUES ($1, $2, 'member') \
             ON CONFLICT (realm_id, user_id) DO NOTHING",
        )
        .bind(realm_id)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Realm summary *for a member* (None if the user isn't a member).
    pub async fn summary_for(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<RealmSummary>> {
        let row: Option<SummaryRow> = sqlx::query_as(&format!(
            "SELECT {SUMMARY_COLS} FROM realms r JOIN realm_members m ON m.realm_id = r.id \
             WHERE r.id = $1 AND m.user_id = $2",
        ))
        .bind(realm_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_summary))
    }

    /// Realms the user belongs to (owned or joined), newest first.
    pub async fn list_mine(&self, user_id: &str) -> anyhow::Result<Vec<RealmSummary>> {
        let rows: Vec<SummaryRow> = sqlx::query_as(&format!(
            "SELECT {SUMMARY_COLS} FROM realms r JOIN realm_members m ON m.realm_id = r.id \
             WHERE m.user_id = $1 ORDER BY r.created_at DESC",
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_summary).collect())
    }

    /// Current manifest + version for a member (None if not a member).
    pub async fn manifest(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<RealmManifest>> {
        let row: Option<(String, i32)> = sqlx::query_as(
            "SELECT r.manifest::text, r.manifest_version FROM realms r \
             JOIN realm_members m ON m.realm_id = r.id WHERE r.id = $1 AND m.user_id = $2",
        )
        .bind(realm_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((json, version)) = row else { return Ok(None) };
        let mut manifest: RealmManifest = serde_json::from_str(&json).unwrap_or_default();
        manifest.version = version;
        Ok(Some(manifest))
    }

    /// Replace the manifest + bump version. Requires `owner`/`admin`. Returns
    /// the new version, or `None` if not permitted / realm gone.
    pub async fn push_manifest(
        &self,
        realm_id: &str,
        actor_id: &str,
        manifest: &RealmManifest,
    ) -> anyhow::Result<Option<i32>> {
        let json = serde_json::to_string(manifest)?;
        let row: Option<(i32,)> = sqlx::query_as(
            "UPDATE realms SET manifest = $3::jsonb, manifest_version = manifest_version + 1, \
                    mc_version = $4, loader = $5 \
             WHERE id = $1 AND EXISTS ( \
                 SELECT 1 FROM realm_members WHERE realm_id = $1 AND user_id = $2 AND role IN ('owner','admin')) \
             RETURNING manifest_version",
        )
        .bind(realm_id)
        .bind(actor_id)
        .bind(&json)
        .bind(&manifest.mc_version)
        .bind(&manifest.loader)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(v,)| v))
    }

    /// Member list for a realm (only if `user_id` is a member).
    pub async fn members(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<Vec<MemberInfo>>> {
        if self.role_of(realm_id, user_id).await?.is_none() {
            return Ok(None);
        }
        let rows: Vec<MemberRow> = sqlx::query_as(
            "SELECT m.user_id, u.username, m.role, m.synced_version, m.joined_at::text \
             FROM realm_members m JOIN users u ON u.id = m.user_id \
             WHERE m.realm_id = $1 ORDER BY m.joined_at",
        )
        .bind(realm_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(
            rows.into_iter()
                .map(|(user_id, username, role, synced_version, joined_at)| MemberInfo {
                    user_id,
                    username,
                    role,
                    synced_version,
                    joined_at,
                })
                .collect(),
        ))
    }

    /// Owner sets another member's role (`admin`/`member`); never touches the
    /// owner row. Returns whether a row changed.
    pub async fn set_role(
        &self,
        realm_id: &str,
        actor_id: &str,
        target_id: &str,
        role: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "UPDATE realm_members SET role = $3 \
             WHERE realm_id = $1 AND user_id = $4 AND role <> 'owner' AND EXISTS ( \
                 SELECT 1 FROM realm_members WHERE realm_id = $1 AND user_id = $2 AND role = 'owner')",
        )
        .bind(realm_id)
        .bind(actor_id)
        .bind(role)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Self-leave, or owner removes a member. The owner can't leave (disband
    /// instead). Returns whether a row was removed.
    pub async fn leave_or_remove(
        &self,
        realm_id: &str,
        actor_id: &str,
        target_id: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "DELETE FROM realm_members WHERE realm_id = $1 AND user_id = $3 AND role <> 'owner' AND ( \
                 $2 = $3 OR EXISTS (SELECT 1 FROM realm_members WHERE realm_id = $1 AND user_id = $2 AND role = 'owner'))",
        )
        .bind(realm_id)
        .bind(actor_id)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Member records the version it has synced to (lets the owner see progress).
    pub async fn mark_synced(&self, realm_id: &str, user_id: &str, version: i32) -> anyhow::Result<()> {
        sqlx::query("UPDATE realm_members SET synced_version = $3 WHERE realm_id = $1 AND user_id = $2")
            .bind(realm_id)
            .bind(user_id)
            .bind(version)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Owner disbands the realm (cascades members). Returns whether it existed.
    pub async fn delete(&self, realm_id: &str, actor_id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM realms WHERE id = $1 AND owner_id = $2")
            .bind(realm_id)
            .bind(actor_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn role_of(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT role FROM realm_members WHERE realm_id = $1 AND user_id = $2")
                .bind(realm_id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(r,)| r))
    }

    /// Whether `user_id` may push content to the realm (owner/admin).
    pub async fn can_push(&self, realm_id: &str, user_id: &str) -> anyhow::Result<bool> {
        Ok(matches!(self.role_of(realm_id, user_id).await?.as_deref(), Some("owner") | Some("admin")))
    }

    /// Whether `user_id` is a member of the realm (can read content).
    pub async fn is_member(&self, realm_id: &str, user_id: &str) -> anyhow::Result<bool> {
        Ok(self.role_of(realm_id, user_id).await?.is_some())
    }

    /// Publish (or heartbeat) `user_id` as the realm's host, reachable at `address`.
    /// Caller must authorize membership first. Overwrites any previous host
    /// (last writer wins) and stamps `host_at = NOW()` as the heartbeat.
    pub async fn set_host(&self, realm_id: &str, user_id: &str, address: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE realms SET host_address = $3, host_user_id = $2, host_at = NOW() WHERE id = $1",
        )
        .bind(realm_id)
        .bind(user_id)
        .bind(address)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The realm's *fresh* host (heartbeat within the last 90s) + its username,
    /// or `(None, None)` if nobody is hosting / the heartbeat went stale.
    pub async fn get_host(&self, realm_id: &str) -> anyhow::Result<(Option<String>, Option<String>)> {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT r.host_address, u.username \
             FROM realms r LEFT JOIN users u ON u.id = r.host_user_id \
             WHERE r.id = $1 AND r.host_at IS NOT NULL AND r.host_at > NOW() - INTERVAL '90 seconds'",
        )
        .bind(realm_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.unwrap_or((None, None)))
    }

    /// Stop hosting: clear the host record **iff** `user_id` is the current host.
    /// Returns whether a row changed.
    pub async fn clear_host(&self, realm_id: &str, user_id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "UPDATE realms SET host_address = NULL, host_user_id = NULL, host_at = NULL \
             WHERE id = $1 AND host_user_id = $2",
        )
        .bind(realm_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }
}

/* ---- handlers ---- */

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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn mk_user(pool: &PgPool, id: &str, email: &str) {
        sqlx::query("INSERT INTO users (id, email) VALUES ($1, $2) ON CONFLICT (id) DO NOTHING")
            .bind(id)
            .bind(email)
            .execute(pool)
            .await
            .unwrap();
    }

    fn req(name: &str) -> CreateRealmReq {
        CreateRealmReq {
            name: name.into(),
            expires_in_secs: None,
            manifest: RealmManifest { mc_version: Some("1.20.1".into()), loader: Some("fabric".into()), ..Default::default() },
        }
    }

    #[tokio::test]
    async fn realm_lifecycle_and_permissions() {
        let Some(pool) = crate::db::test_pool().await else { return };
        // Clean slate for this test's fixed users (cascades realms/members).
        sqlx::query("DELETE FROM users WHERE id IN ('t-realm-owner', 't-realm-friend')")
            .execute(&pool)
            .await
            .unwrap();
        mk_user(&pool, "t-realm-owner", "owner@test.local").await;
        mk_user(&pool, "t-realm-friend", "friend@test.local").await;

        let store = RealmStore::new(pool.clone());

        // create → owner enrolled, manifest at v1
        let realm = store.create("t-realm-owner", &req("Survival")).await.unwrap();
        assert_eq!(realm.role, "owner");
        assert_eq!(realm.manifest_version, 1);
        assert_eq!(realm.mc_version.as_deref(), Some("1.20.1"));
        assert_eq!(realm.code.len(), 6);

        // join by code → friend is a member
        let joined = store.join("t-realm-friend", &realm.code).await.unwrap().unwrap();
        assert_eq!(joined.id, realm.id);
        assert_eq!(joined.role, "member");
        // unknown code → None
        assert!(store.join("t-realm-friend", "ZZZZZZ").await.unwrap().is_none());

        // member CANNOT push the manifest
        let m = RealmManifest {
            mc_version: Some("1.20.1".into()),
            loader: Some("fabric".into()),
            loader_version: None,
            files: vec![RealmFile {
                path: "mods/sodium.jar".into(),
                sha1: Some("abc".into()),
                sha512: None,
                size: Some(10),
                url: Some("https://cdn/sodium.jar".into()),
                source: Some("modrinth".into()),
            }],
            overrides: None,
            source: None,
            version: 0,
        };
        assert!(store.push_manifest(&realm.id, "t-realm-friend", &m).await.unwrap().is_none());

        // owner CAN push → version bumps to 2
        assert_eq!(store.push_manifest(&realm.id, "t-realm-owner", &m).await.unwrap(), Some(2));

        // member reads the manifest (files + server version)
        let got = store.manifest(&realm.id, "t-realm-friend").await.unwrap().unwrap();
        assert_eq!(got.version, 2);
        assert_eq!(got.files.len(), 1);
        assert_eq!(got.files[0].path, "mods/sodium.jar");

        // promote friend → admin; now they can push
        assert!(store.set_role(&realm.id, "t-realm-owner", "t-realm-friend", "admin").await.unwrap());
        assert_eq!(store.push_manifest(&realm.id, "t-realm-friend", &m).await.unwrap(), Some(3));

        // members list shows both; mark-synced records progress
        store.mark_synced(&realm.id, "t-realm-friend", 3).await.unwrap();
        let members = store.members(&realm.id, "t-realm-owner").await.unwrap().unwrap();
        assert_eq!(members.len(), 2);
        assert!(members.iter().any(|m| m.user_id == "t-realm-friend" && m.synced_version == 3));

        // host publish (P3): nobody hosting yet → (None, None)
        assert_eq!(store.get_host(&realm.id).await.unwrap(), (None, None));
        // owner publishes a host address → fresh read returns it
        store.set_host(&realm.id, "t-realm-owner", "10.144.0.1:52137").await.unwrap();
        let (addr, _name) = store.get_host(&realm.id).await.unwrap();
        assert_eq!(addr.as_deref(), Some("10.144.0.1:52137"));
        // a non-host can't clear it; the host can, and then it reads empty again
        assert!(!store.clear_host(&realm.id, "t-realm-friend").await.unwrap());
        assert!(store.clear_host(&realm.id, "t-realm-owner").await.unwrap());
        assert_eq!(store.get_host(&realm.id).await.unwrap(), (None, None));

        // friend leaves → 1 member; owner can't be removed
        assert!(store.leave_or_remove(&realm.id, "t-realm-friend", "t-realm-friend").await.unwrap());
        assert!(!store.leave_or_remove(&realm.id, "t-realm-owner", "t-realm-owner").await.unwrap());
        assert_eq!(store.list_mine("t-realm-friend").await.unwrap().len(), 0);

        // non-owner can't disband; owner can
        assert!(!store.delete(&realm.id, "t-realm-friend").await.unwrap());
        assert!(store.delete(&realm.id, "t-realm-owner").await.unwrap());
        assert!(store.summary_for(&realm.id, "t-realm-owner").await.unwrap().is_none());

        // cleanup
        sqlx::query("DELETE FROM users WHERE id IN ('t-realm-owner', 't-realm-friend')")
            .execute(&pool)
            .await
            .unwrap();
    }
}
