//! Session validation for our own (non-better-auth) endpoints.
//!
//! better-auth owns the `sessions` table; its session cookie
//! (`better-auth.session-token`) carries the **raw** token (== `sessions.token`,
//! no appended signature — see better-auth-core `cookie_utils`), so we validate
//! by a direct, version-stable DB lookup instead of reimplementing its cookie
//! handling. Both the cookie and `Authorization: Bearer <token>` are accepted
//! (the launcher's `ServerClient` sends the cookie via its cookie store).

use axum::http::{header, HeaderMap, StatusCode};
use sqlx::PgPool;

const COOKIE_NAME: &str = "better-auth.session-token";

/// Pull the session token from the request: prefer the better-auth cookie, fall
/// back to an `Authorization: Bearer` header.
fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(cookie) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in cookie.split(';') {
            if let Some(v) = part.trim().strip_prefix(&format!("{COOKIE_NAME}=")) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Resolve the authenticated `user_id` for the request, or a `401`. Validates
/// the session is active and unexpired against the better-auth `sessions` table.
pub async fn require_user(pool: &PgPool, headers: &HeaderMap) -> Result<String, StatusCode> {
    let token = token_from_headers(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT user_id FROM sessions WHERE token = $1 AND active = TRUE AND expires_at > NOW()",
    )
    .bind(&token)
    .fetch_optional(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    row.map(|(uid,)| uid).ok_or(StatusCode::UNAUTHORIZED)
}
