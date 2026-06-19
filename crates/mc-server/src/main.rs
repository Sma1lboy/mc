//! mc-launcher lite backend (axum 0.8). Loader/version meta aggregation, news,
//! instance sharing, and **better-auth** launcher accounts. Runs on
//! `127.0.0.1:8787` by default (override with `PORT`).
//!
//! Local test env:  set `DATABASE_URL` (Supabase) then `cargo run -p mc-server`.

mod auth;
mod db;
mod meta;
mod news;
mod share;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use better_auth::handlers::AxumIntegration;
use tower_http::cors::CorsLayer;

use share::{ShareStore, SharedInstance};

#[derive(Clone)]
struct AppState {
    http: reqwest::Client,
    shares: ShareStore,
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
        shares: ShareStore::new(pool),
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
        .with_state(state);

    let app = Router::new()
        .nest("/v1/auth", auth_router)
        .merge(api)
        .layer(CorsLayer::permissive());

    let port = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8787u16);
    let addr = format!("127.0.0.1:{port}");
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
