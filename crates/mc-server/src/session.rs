//! Session validation for our own (non-better-auth) endpoints.
//!
//! better-auth owns the `sessions` table; its session cookie
//! (`better-auth.session-token`) carries the **raw** token (== `sessions.token`,
//! no appended signature — see better-auth-core `cookie_utils`), so we validate
//! by a direct, version-stable DB lookup instead of reimplementing its cookie
//! handling. Both the cookie and `Authorization: Bearer <token>` are accepted
//! (the launcher's `ServerClient` sends the cookie via its cookie store).

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
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

/// The authenticated user id for the current request. Handlers behind
/// [`auth_middleware`] declare `user: AuthUser` as a parameter — the session was
/// already validated once by the middleware, so extraction is a free extensions
/// lookup (a `401` outside the middleware, so a forgotten route grouping fails
/// closed instead of panicking).
#[derive(Clone)]
pub struct AuthUser(pub String);

impl<S: Send + Sync> FromRequestParts<S> for AuthUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, StatusCode> {
        parts.extensions.get::<AuthUser>().cloned().ok_or(StatusCode::UNAUTHORIZED)
    }
}

/// Default-deny auth layer: every route behind it requires a valid session
/// (`401` otherwise). Validates once and stashes [`AuthUser`] in request
/// extensions for handlers. New endpoints are authed by default — a route is
/// public only by being explicitly placed in the public router in `main.rs`.
pub async fn auth_middleware(
    State(state): State<crate::AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let user_id = require_user(&state.pool, req.headers()).await?;
    req.extensions_mut().insert(AuthUser(user_id));
    Ok(next.run(req).await)
}
