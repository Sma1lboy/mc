//! Notifications — a typed, server-backed inbox the launcher polls (the bell in
//! the top bar). Producers across the API (`friend.rs`, `realm.rs`) emit rows via
//! [`NotificationStore::create`]; the client lists the last 50 and marks them all
//! read when the dropdown opens.
//!
//! `kind` is one of `friend_request` | `friend_accepted` | `realm_invite`.
//! Actor + realm names are resolved at read time (left joins) so the client gets
//! flat, ready-to-render fields. `created_at` is serialized as text (no chrono).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::Serialize;
use sqlx::PgPool;

use crate::session::require_user;
use crate::AppState;

/// A notification as seen by a client — flat, with actor + realm names resolved.
#[derive(Serialize)]
pub struct Notification {
    pub id: String,
    pub kind: String,
    pub actor_id: Option<String>,
    pub actor_username: Option<String>,
    pub realm_id: Option<String>,
    pub realm_name: Option<String>,
    pub created_at: String,
    pub read: bool,
}

/// Row shape for the list query (mirrors the SELECT column order).
#[derive(sqlx::FromRow)]
struct NotificationRow {
    id: String,
    kind: String,
    actor_id: Option<String>,
    actor_username: Option<String>,
    realm_id: Option<String>,
    realm_name: Option<String>,
    created_at: String,
    read: bool,
}

#[derive(Clone)]
pub struct NotificationStore {
    pool: PgPool,
}

impl NotificationStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Insert a notification addressed to `user_id`. `id`/`created_at` default.
    pub async fn create(
        &self,
        user_id: &str,
        kind: &str,
        actor_id: Option<&str>,
        realm_id: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO notifications (user_id, kind, actor_id, realm_id) VALUES ($1,$2,$3,$4)")
            .bind(user_id)
            .bind(kind)
            .bind(actor_id)
            .bind(realm_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The user's last 50 notifications (newest first), actor + realm names resolved.
    pub async fn list(&self, user_id: &str) -> anyhow::Result<Vec<Notification>> {
        let rows: Vec<NotificationRow> = sqlx::query_as(
            "SELECT n.id, n.kind, n.actor_id, u.username AS actor_username, \
                    n.realm_id, r.name AS realm_name, \
                    n.created_at::text AS created_at, (n.read_at IS NOT NULL) AS read \
             FROM notifications n \
             LEFT JOIN users u  ON u.id = n.actor_id \
             LEFT JOIN realms r ON r.id = n.realm_id \
             WHERE n.user_id = $1 ORDER BY n.created_at DESC LIMIT 50",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Notification {
                id: r.id,
                kind: r.kind,
                actor_id: r.actor_id,
                actor_username: r.actor_username,
                realm_id: r.realm_id,
                realm_name: r.realm_name,
                created_at: r.created_at,
                read: r.read,
            })
            .collect())
    }

    /// Mark every unread notification for the user as read.
    pub async fn mark_all_read(&self, user_id: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE notifications SET read_at = NOW() WHERE user_id=$1 AND read_at IS NULL")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn ise(_: anyhow::Error) -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

/// `GET /v1/notifications` — the current user's last 50 notifications.
pub async fn list(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<Notification>>, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    s.notifications.list(&me).await.map(Json).map_err(ise)
}

/// `POST /v1/notifications/read` — mark all of the current user's as read.
pub async fn read_all(State(s): State<AppState>, headers: HeaderMap) -> Result<StatusCode, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    s.notifications.mark_all_read(&me).await.map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}
