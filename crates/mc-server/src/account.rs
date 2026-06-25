//! Account linking — bind extra identities (notably **Microsoft**) to a kobeMC
//! user, on top of better-auth's own email/password identity. Microsoft login
//! happens in the launcher (device-code); once the user is in a kobeMC session
//! the launcher posts the verified MS identity here and we record the link in
//! better-auth's `accounts` table (`provider_id='microsoft'`, `account_id=<ms
//! uuid>`). One kobeMC user ↔ many identities.
//!
//! NOTE (MVP trust model): we trust the launcher's claimed MS identity (it just
//! completed the real device-code + Xbox/Minecraft flow). Hardening — verifying
//! the Minecraft access token server-side against Mojang services — is a later
//! step; the link can be re-pointed, never silently spoofs another user's login.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::session::require_user;
use crate::AppState;

/// Request to bind a Microsoft identity to the current kobeMC user.
#[derive(Deserialize)]
pub struct LinkMicrosoftReq {
    /// The Minecraft profile UUID (stable per Microsoft account).
    pub account_id: String,
    /// Display gamertag / MC username at link time (optional, informational).
    #[serde(default)]
    pub username: Option<String>,
}

/// One linked identity of a user (e.g. `credential` email, or `microsoft`).
#[derive(Serialize)]
pub struct Identity {
    pub provider: String,
    pub account_id: String,
}

/// Insert/repoint a `(provider, account_id)` link onto `user_id`.
async fn link(pool: &PgPool, user_id: &str, provider: &str, account_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO accounts (id, account_id, provider_id, user_id) \
         VALUES (gen_random_uuid()::text, $1, $2, $3) \
         ON CONFLICT (provider_id, account_id) DO UPDATE SET user_id = EXCLUDED.user_id, updated_at = NOW()",
    )
    .bind(account_id)
    .bind(provider)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn identities(pool: &PgPool, user_id: &str) -> anyhow::Result<Vec<Identity>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT provider_id, account_id FROM accounts WHERE user_id = $1 ORDER BY provider_id")
            .bind(user_id)
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(provider, account_id)| Identity { provider, account_id }).collect())
}

async fn unlink(pool: &PgPool, user_id: &str, provider: &str) -> anyhow::Result<u64> {
    let res = sqlx::query("DELETE FROM accounts WHERE user_id = $1 AND provider_id = $2")
        .bind(user_id)
        .bind(provider)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/* ---- handlers ---- */

/// `POST /v1/account/link/microsoft` — bind a Microsoft identity (authed).
pub async fn link_microsoft(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LinkMicrosoftReq>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    let _ = req.username; // informational only in the MVP
    link(&s.pool, &user, "microsoft", &req.account_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/account/identities` — list the current user's linked identities.
pub async fn list_identities(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Identity>>, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    identities(&s.pool, &user).await.map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// `DELETE /v1/account/link/{provider}` — unlink a provider (authed).
pub async fn unlink_provider(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let user = require_user(&s.pool, &headers).await?;
    let n = unlink(&s.pool, &user, &provider).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if n > 0 { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::NOT_FOUND) }
}
