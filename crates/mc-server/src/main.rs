//! mc-launcher lite backend (axum 0.8). Loader/version meta aggregation, news,
//! instance sharing, and **better-auth** launcher accounts. Runs on
//! `127.0.0.1:8787` by default (override with `PORT`).
//!
//! Local test env:  set `DATABASE_URL` (Supabase) then `cargo run -p mc-server`.

mod account;
mod auth;
mod db;
mod friend;
mod meta;
mod news;
mod notification;
mod realm;
mod session;
mod share;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use better_auth::handlers::AxumIntegration;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;

use friend::FriendStore;
use notification::NotificationStore;
use realm::RealmStore;
use share::{ShareStore, SharedInstance};

#[derive(Clone)]
struct AppState {
    http: reqwest::Client,
    /// Shared Supabase pool — also used directly for session validation
    /// (`session::require_user`) on our authed endpoints.
    pool: PgPool,
    shares: ShareStore,
    realms: RealmStore,
    friends: FriendStore,
    notifications: NotificationStore,
    /// Directory holding per-realm overrides zip blobs (a mounted volume in prod,
    /// `BLOB_DIR`; defaults to `./blobs` for local dev).
    blob_dir: std::path::PathBuf,
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // One Supabase pool, shared by better-auth (users/sessions/accounts) and our
    // own share store.
    let pool = db::connect().await.expect("connect database");

    let auth = auth::build(pool.clone()).await.expect("build better-auth");

    let state = AppState {
        http: reqwest::Client::builder()
            .user_agent("mc-launcher-server/0.1")
            .build()
            .expect("build http client"),
        pool: pool.clone(),
        shares: ShareStore::new(pool.clone()),
        realms: RealmStore::new(pool.clone()),
        notifications: NotificationStore::new(pool.clone()),
        friends: FriendStore::new(pool),
        blob_dir: std::env::var("BLOB_DIR").unwrap_or_else(|_| "blobs".to_string()).into(),
    };

    // better-auth's own routes (sign-up/email, sign-in/email, get-session,
    // sign-out, …), mounted under /v1/auth.
    let auth_router: Router = auth.clone().axum_router().with_state(auth);

    // Our own routes (separate state).
    let api: Router = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/meta/versions", get(versions))
        .route("/v1/meta/loaders/{mc_version}", get(loaders))
        .route("/v1/news", get(get_news))
        .route("/v1/instances/share", post(share_instance))
        .route("/v1/instances/{id}", get(get_instance))
        // Public agent chat-transcript sharing (no auth; reuses the shares table).
        .route("/v1/agent/conversations", post(share_conversation))
        .route("/v1/agent/conversations/{id}", get(get_conversation))
        // Account linking (bind Microsoft to a kobeMC user; authed).
        .route("/v1/account/link/microsoft", post(account::link_microsoft))
        .route("/v1/account/identities", get(account::list_identities))
        .route("/v1/account/link/{provider}", delete(account::unlink_provider))
        // Private realms + mod sync (authed).
        .route("/v1/realms", post(realm::create))
        .route("/v1/realms/join", post(realm::join))
        .route("/v1/realms/mine", get(realm::list_mine))
        .route("/v1/realms/{id}", get(realm::get).delete(realm::disband))
        .route("/v1/realms/{id}/manifest", get(realm::get_manifest).post(realm::push_manifest))
        .route("/v1/realms/{id}/members", get(realm::members))
        .route("/v1/realms/{id}/invite", post(realm::invite))
        .route("/v1/realms/{id}/members/{uid}/role", post(realm::set_role))
        .route("/v1/realms/{id}/members/{uid}", delete(realm::remove_member))
        .route("/v1/realms/{id}/synced", post(realm::mark_synced))
        .route("/v1/realms/{id}/overrides", get(realm::get_overrides).post(realm::upload_overrides))
        .route("/v1/realms/{id}/lobby", get(realm::lobby))
        .route(
            "/v1/realms/{id}/host",
            post(realm::set_host).get(realm::get_host).delete(realm::clear_host),
        )
        // Friends (username search + request/accept; authed).
        .route("/v1/account/username", post(friend::set_username))
        .route("/v1/users/search", get(friend::search))
        .route("/v1/friends", get(friend::list))
        .route("/v1/friends/request", post(friend::request))
        .route("/v1/friends/requests", get(friend::requests))
        .route("/v1/friends/accept", post(friend::accept))
        .route("/v1/friends/decline", post(friend::decline))
        .route("/v1/friends/{user_id}", delete(friend::remove))
        // Presence heartbeat (online status + current activity; authed).
        .route("/v1/presence", post(friend::presence))
        // Notifications inbox (friend requests/accepts + realm invites; authed).
        .route("/v1/notifications", get(notification::list))
        .route("/v1/notifications/read", post(notification::read_all))
        .with_state(state);

    let app = Router::new()
        .nest("/v1/auth", auth_router)
        .merge(api)
        // Overrides blobs can be large (config + non-CDN content); raise the body cap.
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024))
        .layer(CorsLayer::permissive());

    let port = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8787u16);
    // Bind 0.0.0.0 on deployed/release builds so the platform's router can reach
    // the container; 127.0.0.1 in dev. Override with HOST.
    let host = std::env::var("HOST").unwrap_or_else(|_| {
        if cfg!(debug_assertions) { "127.0.0.1".to_string() } else { "0.0.0.0".to_string() }
    });
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!("mc-server listening on http://{addr}");
    axum::serve(listener, app).await.expect("serve");
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "service": "mc-server", "version": env!("CARGO_PKG_VERSION") }))
}

async fn versions(State(s): State<AppState>) -> Json<Vec<meta::VersionEntry>> {
    Json(meta::versions(&s.http).await)
}

async fn loaders(State(s): State<AppState>, Path(mc): Path<String>) -> Json<meta::LoaderMeta> {
    Json(meta::loaders_for(&s.http, &mc).await)
}

async fn get_news() -> Json<Vec<news::NewsItem>> {
    Json(news::feed())
}

async fn share_instance(
    State(s): State<AppState>,
    Json(inst): Json<SharedInstance>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let id = s.shares.put(inst).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "id": id })))
}

async fn get_instance(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SharedInstance>, StatusCode> {
    s.shares.get(&id).await.map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// Publish an agent chat transcript (opaque JSON: the UIMessage[] + optional
/// title/model) and get a public id. Body is stored as-is; capped at ~1 MiB.
async fn share_conversation(
    State(s): State<AppState>,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if serde_json::to_string(&payload).map(|j| j.len()).unwrap_or(usize::MAX) > 1_048_576 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let id = s
        .shares
        .put_raw(&payload)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(serde_json::json!({ "id": id })))
}

/// Fetch a shared conversation by id (public, no auth). Content-negotiated:
/// a browser (`Accept: text/html`) gets a rendered conversation page that mirrors
/// the desktop chat UI; anything else gets the raw JSON payload.
async fn get_conversation(
    State(s): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(value) = s.shares.get_raw(&id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let wants_html = headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/html"));
    if wants_html {
        Html(share::render_conversation_html(&value)).into_response()
    } else {
        Json(value).into_response()
    }
}
