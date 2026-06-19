//! Launcher accounts via **better-auth** (the Rust port of @better-auth) — the
//! most complete auth framework in the ecosystem. We share our Supabase pool
//! with it (`SqlxAdapter::from_pool`) and mount its router; it owns the
//! users/sessions/accounts schema and the sign-up/sign-in/session endpoints.
//!
//! Only Email/Password + Session are enabled now; social login, passkeys, 2FA,
//! organizations etc. are additional `.plugin(...)` lines when wanted.

use std::sync::Arc;

use better_auth::adapters::SqlxAdapter;
use better_auth::plugins::{EmailPasswordPlugin, SessionManagementPlugin};
use better_auth::{AuthBuilder, AuthConfig, BetterAuth};
use sqlx::PgPool;

/// The concrete better-auth instance type (backed by our shared sqlx pool).
pub type Auth = Arc<BetterAuth<SqlxAdapter>>;

/// Build the better-auth instance on a shared Supabase pool.
pub async fn build(pool: PgPool) -> anyhow::Result<Auth> {
    // 32+ char signing secret. Set AUTH_SECRET in prod; dev default is fine for
    // the throwaway dev DB.
    let secret = std::env::var("AUTH_SECRET")
        .unwrap_or_else(|_| "dev-only-insecure-secret-change-me-0123456789".to_string());
    let base_url =
        std::env::var("AUTH_BASE_URL").unwrap_or_else(|_| "http://127.0.0.1:8787".to_string());

    let config = AuthConfig::new(secret).base_url(base_url).password_min_length(8);

    let auth = AuthBuilder::new(config)
        .database(SqlxAdapter::from_pool(pool))
        .plugin(EmailPasswordPlugin::new().enable_signup(true))
        .plugin(SessionManagementPlugin::new())
        .build()
        .await?;

    Ok(Arc::new(auth))
}
