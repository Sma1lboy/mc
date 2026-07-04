//! Agent conversation history — authed, per-user CRUD over `agent_conversations`.
//!
//! Unlike `share.rs` (public, content-addressed snapshots), these rows are the
//! user's private chat archive: keyed `(user_id, id)` with the client-minted
//! conversation id, newest-wins by the client's `updatedAt` clock. The body is
//! stored opaquely (the desktop's ConversationRecord JSON) so the schema never
//! chases the UIMessage shape.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::{session, AppState};

/// Payload cap — same 1 MiB as conversation shares.
const MAX_BYTES: usize = 1_048_576;

#[derive(Serialize)]
pub struct ConversationHead {
    pub id: String,
    pub title: String,
    pub updated_at_ms: i64,
}

/// List the user's conversations, newest first (heads only — no message bodies).
pub async fn list(
    State(s): State<AppState>,
    user: session::AuthUser,
) -> Result<Json<Vec<ConversationHead>>, StatusCode> {
    let rows: Vec<(String, String, i64)> = sqlx::query_as(
        "SELECT id, title, updated_at_ms FROM agent_conversations
         WHERE user_id = $1 ORDER BY updated_at_ms DESC LIMIT 200",
    )
    .bind(&user.0)
    .fetch_all(&s.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, title, updated_at_ms)| ConversationHead { id, title, updated_at_ms })
            .collect(),
    ))
}

/// Fetch one conversation's full record (the stored JSON, as-is).
pub async fn get_one(
    State(s): State<AppState>,
    user: session::AuthUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT json FROM agent_conversations WHERE user_id = $1 AND id = $2",
    )
    .bind(&user.0)
    .bind(&id)
    .fetch_optional(&s.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (json,) = row.ok_or(StatusCode::NOT_FOUND)?;
    serde_json::from_str(&json).map(Json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Upsert one conversation. `title`/`updatedAt` are lifted out of the record for
/// cheap listing/merging; the record itself is stored opaquely.
pub async fn put_one(
    State(s): State<AppState>,
    user: session::AuthUser,
    Path(id): Path<String>,
    Json(record): Json<serde_json::Value>,
) -> Result<StatusCode, StatusCode> {
    let json = serde_json::to_string(&record).map_err(|_| StatusCode::BAD_REQUEST)?;
    if json.len() > MAX_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let title = record.get("title").and_then(|t| t.as_str()).unwrap_or("");
    let updated_at_ms = record.get("updatedAt").and_then(|t| t.as_i64()).unwrap_or(0);
    sqlx::query(
        "INSERT INTO agent_conversations (user_id, id, title, updated_at_ms, json)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, id) DO UPDATE
           SET title = EXCLUDED.title, updated_at_ms = EXCLUDED.updated_at_ms, json = EXCLUDED.json",
    )
    .bind(&user.0)
    .bind(&id)
    .bind(title)
    .bind(updated_at_ms)
    .bind(&json)
    .execute(&s.pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete one conversation (idempotent — deleting a missing row is fine).
pub async fn delete_one(
    State(s): State<AppState>,
    user: session::AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    sqlx::query("DELETE FROM agent_conversations WHERE user_id = $1 AND id = $2")
        .bind(&user.0)
        .bind(&id)
        .execute(&s.pool)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn upsert_and_roundtrip() {
        let Some(pool) = crate::db::test_pool().await else { return };
        sqlx::query("INSERT INTO users (id, name) VALUES ('hist-test-user', 'tester') ON CONFLICT (id) DO NOTHING")
            .execute(&pool)
            .await
            .unwrap();
        let record = serde_json::json!({
            "id": "chat-1", "title": "test pack", "updatedAt": 42, "messages": []
        });
        let json = serde_json::to_string(&record).unwrap();
        sqlx::query(
            "INSERT INTO agent_conversations (user_id, id, title, updated_at_ms, json)
             VALUES ('hist-test-user', 'chat-1', 'test pack', 42, $1)
             ON CONFLICT (user_id, id) DO UPDATE SET json = EXCLUDED.json",
        )
        .bind(&json)
        .execute(&pool)
        .await
        .unwrap();
        let row: (String, i64) = sqlx::query_as(
            "SELECT title, updated_at_ms FROM agent_conversations WHERE user_id = 'hist-test-user' AND id = 'chat-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row, ("test pack".to_string(), 42));
    }
}
