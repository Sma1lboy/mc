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

/// Public, in-repo placeholder signing secret. Only ever used in debug/dev
/// builds; a release build with `AUTH_SECRET` unset refuses to start (see
/// `resolve_secret`) so sessions are never signed with a known value in prod.
const DEV_INSECURE_SECRET: &str = "dev-only-insecure-secret-change-me-0123456789";

/// Resolve the session-signing secret, hard-failing in non-dev builds when it
/// is unset so we never sign sessions with the public in-repo default.
///
/// `raw` is the result of looking up `AUTH_SECRET`; `dev` is the dev-mode
/// signal (debug build). In dev we fall back to a known insecure secret for
/// ergonomics; in a release build the env var is mandatory.
fn resolve_secret(raw: Option<String>, dev: bool) -> anyhow::Result<String> {
    match raw {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ if dev => {
            tracing::warn!(
                "AUTH_SECRET unset — signing sessions with the PUBLIC in-repo dev secret. \
                 Anyone can forge tokens. Set AUTH_SECRET before any non-dev deployment."
            );
            Ok(DEV_INSECURE_SECRET.to_string())
        }
        _ => anyhow::bail!(
            "AUTH_SECRET is required in a release build: set AUTH_SECRET to a private, \
             32+ char random value (e.g. `openssl rand -base64 32`). Refusing to start \
             rather than sign sessions with a public secret."
        ),
    }
}

/// Build the better-auth instance on a shared Supabase pool.
pub async fn build(pool: PgPool) -> anyhow::Result<Auth> {
    // 32+ char signing secret. Set AUTH_SECRET in prod; debug builds fall back
    // to a (loudly warned) insecure dev secret. Release builds without it fail.
    let secret = resolve_secret(std::env::var("AUTH_SECRET").ok(), cfg!(debug_assertions))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_without_secret_fails() {
        assert!(resolve_secret(None, false).is_err());
        assert!(resolve_secret(Some("   ".into()), false).is_err());
    }

    #[test]
    fn release_with_secret_uses_it() {
        assert_eq!(resolve_secret(Some("real-prod-secret".into()), false).unwrap(), "real-prod-secret");
    }

    #[test]
    fn dev_without_secret_falls_back() {
        assert_eq!(resolve_secret(None, true).unwrap(), DEV_INSECURE_SECRET);
    }

    #[test]
    fn dev_with_secret_prefers_it() {
        assert_eq!(resolve_secret(Some("explicit".into()), true).unwrap(), "explicit");
    }
}
