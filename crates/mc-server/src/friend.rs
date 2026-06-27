//! Friends — username search + request/accept. Directed `friendships` rows
//! (see `db.rs` FRIENDS_SQL): a request is `(requester, addressee, 'pending')`;
//! the addressee accepts → `'accepted'`. A reverse pending request auto-accepts
//! (mutual add). Search needs a username, which signup leaves null — set it via
//! [`set_username`].

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::session::require_user;
use crate::AppState;

/// A minimal public view of a user (for search results / pending requests).
#[derive(Serialize)]
pub struct UserBrief {
    pub id: String,
    pub username: Option<String>,
}

/// A friend with presence: online (seen within the last 120s) + their current
/// activity (only surfaced while online). Returned by `GET /v1/friends`.
#[derive(Serialize)]
pub struct FriendStatus {
    pub id: String,
    pub username: Option<String>,
    pub online: bool,
    pub activity: Option<String>,
}

#[derive(Deserialize)]
pub struct PresenceReq {
    #[serde(default)]
    pub activity: Option<String>,
}

#[derive(Deserialize)]
pub struct SearchQ {
    pub q: String,
}

#[derive(Deserialize)]
pub struct UserIdReq {
    pub user_id: String,
}

#[derive(Deserialize)]
pub struct UsernameReq {
    pub username: String,
}

#[derive(Clone)]
pub struct FriendStore {
    pool: PgPool,
}

impl FriendStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Prefix-search users by username (case-insensitive), excluding self.
    pub async fn search(&self, me: &str, q: &str) -> anyhow::Result<Vec<UserBrief>> {
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id, username FROM users WHERE username ILIKE $1 AND id <> $2 ORDER BY username LIMIT 20",
        )
        .bind(format!("{q}%"))
        .bind(me)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id, username)| UserBrief { id, username }).collect())
    }

    /// Send a request me→to. A reverse pending request auto-accepts (mutual).
    /// `false` if `to == me` or `to` doesn't exist.
    pub async fn request(&self, me: &str, to: &str) -> anyhow::Result<bool> {
        if me == to {
            return Ok(false);
        }
        let reverse_pending: Option<(String,)> = sqlx::query_as(
            "SELECT requester_id FROM friendships WHERE requester_id=$1 AND addressee_id=$2 AND status='pending'",
        )
        .bind(to)
        .bind(me)
        .fetch_optional(&self.pool)
        .await?;
        if reverse_pending.is_some() {
            sqlx::query("UPDATE friendships SET status='accepted' WHERE requester_id=$1 AND addressee_id=$2")
                .bind(to)
                .bind(me)
                .execute(&self.pool)
                .await?;
            return Ok(true);
        }
        // Insert pending (idempotent). FK violation = `to` doesn't exist → false.
        match sqlx::query(
            "INSERT INTO friendships (requester_id, addressee_id, status) VALUES ($1,$2,'pending') \
             ON CONFLICT (requester_id, addressee_id) DO NOTHING",
        )
        .bind(me)
        .bind(to)
        .execute(&self.pool)
        .await
        {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Accept a pending request from `from` (me is the addressee).
    pub async fn accept(&self, me: &str, from: &str) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "UPDATE friendships SET status='accepted' WHERE requester_id=$1 AND addressee_id=$2 AND status='pending'",
        )
        .bind(from)
        .bind(me)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Decline (delete) a pending request from `from`.
    pub async fn decline(&self, me: &str, from: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM friendships WHERE requester_id=$1 AND addressee_id=$2 AND status='pending'")
            .bind(from)
            .bind(me)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove a friendship in either direction.
    pub async fn remove(&self, me: &str, other: &str) -> anyhow::Result<()> {
        sqlx::query(
            "DELETE FROM friendships WHERE (requester_id=$1 AND addressee_id=$2) OR (requester_id=$2 AND addressee_id=$1)",
        )
        .bind(me)
        .bind(other)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Record a presence heartbeat for `me`: bump `last_seen_at` to now and set
    /// the current `activity` (null = idle).
    pub async fn set_presence(&self, me: &str, activity: Option<&str>) -> anyhow::Result<()> {
        sqlx::query("UPDATE users SET last_seen_at = NOW(), activity = $2 WHERE id = $1")
            .bind(me)
            .bind(activity)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Accepted friends (the other side of each accepted row) with presence:
    /// `online` = seen within the last 120s; `activity` only while online.
    pub async fn list(&self, me: &str) -> anyhow::Result<Vec<FriendStatus>> {
        let rows: Vec<(String, Option<String>, bool, Option<String>)> = sqlx::query_as(
            "SELECT u.id, u.username, \
                    COALESCE(u.last_seen_at > NOW() - interval '120 seconds', FALSE) AS online, \
                    u.activity \
             FROM friendships f \
             JOIN users u ON u.id = CASE WHEN f.requester_id=$1 THEN f.addressee_id ELSE f.requester_id END \
             WHERE f.status='accepted' AND $1 IN (f.requester_id, f.addressee_id) ORDER BY u.username",
        )
        .bind(me)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, username, online, activity)| FriendStatus {
                id,
                username,
                online,
                activity: if online { activity } else { None },
            })
            .collect())
    }

    /// Whether `a` and `b` are accepted friends (either direction).
    pub async fn are_friends(&self, a: &str, b: &str) -> anyhow::Result<bool> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT requester_id FROM friendships WHERE status='accepted' AND \
             ((requester_id=$1 AND addressee_id=$2) OR (requester_id=$2 AND addressee_id=$1)) LIMIT 1",
        )
        .bind(a)
        .bind(b)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.is_some())
    }

    /// Incoming pending requests (requester info).
    pub async fn incoming(&self, me: &str) -> anyhow::Result<Vec<UserBrief>> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT u.id, u.username FROM friendships f JOIN users u ON u.id=f.requester_id \
             WHERE f.addressee_id=$1 AND f.status='pending' ORDER BY f.created_at DESC",
        )
        .bind(me)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id, username)| UserBrief { id, username }).collect())
    }
}

fn ise(_: anyhow::Error) -> StatusCode {
    StatusCode::INTERNAL_SERVER_ERROR
}

/// `GET /v1/users/search?q=` — find users by username prefix.
pub async fn search(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<SearchQ>,
) -> Result<Json<Vec<UserBrief>>, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    s.friends.search(&me, q.q.trim()).await.map(Json).map_err(ise)
}

/// `POST /v1/friends/request` — send a friend request by user id.
pub async fn request(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UserIdReq>,
) -> Result<StatusCode, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    let ok = s.friends.request(&me, &req.user_id).await.map_err(ise)?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::BAD_REQUEST) }
}

/// `GET /v1/friends` — accepted friends with presence (online + activity).
pub async fn list(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<FriendStatus>>, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    s.friends.list(&me).await.map(Json).map_err(ise)
}

/// `POST /v1/presence` — heartbeat the current user's presence + activity.
pub async fn presence(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<PresenceReq>,
) -> Result<StatusCode, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    let activity = req.activity.as_deref().map(str::trim).filter(|a| !a.is_empty());
    s.friends.set_presence(&me, activity).await.map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/friends/requests` — incoming pending requests.
pub async fn requests(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<UserBrief>>, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    s.friends.incoming(&me).await.map(Json).map_err(ise)
}

/// `POST /v1/friends/accept` — accept a request from `user_id`.
pub async fn accept(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UserIdReq>,
) -> Result<StatusCode, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    let ok = s.friends.accept(&me, &req.user_id).await.map_err(ise)?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err(StatusCode::NOT_FOUND) }
}

/// `POST /v1/friends/decline` — decline a request from `user_id`.
pub async fn decline(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UserIdReq>,
) -> Result<StatusCode, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    s.friends.decline(&me, &req.user_id).await.map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/friends/{user_id}` — remove a friend.
pub async fn remove(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    s.friends.remove(&me, &user_id).await.map_err(ise)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/account/username` — set the current user's username (needed for
/// friend search; signup leaves it null). 3–24 chars, `[A-Za-z0-9_-]`.
pub async fn set_username(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UsernameReq>,
) -> Result<StatusCode, StatusCode> {
    let me = require_user(&s.pool, &headers).await?;
    let name = req.username.trim();
    if name.len() < 3
        || name.len() > 24
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    match sqlx::query("UPDATE users SET username=$1, display_username=$1 WHERE id=$2")
        .bind(name)
        .bind(&me)
        .execute(&s.pool)
        .await
    {
        Ok(_) => Ok(StatusCode::NO_CONTENT),
        Err(_) => Err(StatusCode::CONFLICT), // unique violation = taken
    }
}
