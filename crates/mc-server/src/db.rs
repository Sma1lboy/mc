//! Database layer — **Supabase (Postgres)** via sqlx. The connection string
//! comes from `DATABASE_URL` (Supabase Session pooler URI). The pool is shared
//! with better-auth (`SqlxAdapter::from_pool`) for the auth tables, and used
//! directly by `share.rs` for the `shares` table.

use anyhow::Context;
use sqlx::postgres::{PgPool, PgPoolOptions};

/// better-auth core schema (users / sessions / accounts / verifications + idx).
/// Copied from better-auth-rs `migrations/001_create_core_tables.sql` — the
/// SqlxAdapter expects exactly these tables (it has no built-in migrator).
const BETTER_AUTH_CORE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    name TEXT,
    email TEXT UNIQUE,
    email_verified BOOLEAN NOT NULL DEFAULT FALSE,
    image TEXT,
    username TEXT UNIQUE,
    display_username TEXT,
    two_factor_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    role TEXT,
    banned BOOLEAN NOT NULL DEFAULT FALSE,
    ban_reason TEXT,
    ban_expires TIMESTAMPTZ,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL,
    token TEXT NOT NULL UNIQUE,
    ip_address TEXT,
    user_agent TEXT,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    impersonated_by TEXT,
    active_organization_id TEXT,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    access_token TEXT,
    refresh_token TEXT,
    id_token TEXT,
    access_token_expires_at TIMESTAMPTZ,
    refresh_token_expires_at TIMESTAMPTZ,
    scope TEXT,
    password TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(provider_id, account_id)
);
CREATE TABLE IF NOT EXISTS verifications (
    id TEXT PRIMARY KEY,
    identifier TEXT NOT NULL,
    value TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_users_email ON users(email);
CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_sessions_token ON sessions(token);
CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);
CREATE INDEX IF NOT EXISTS idx_accounts_user_id ON accounts(user_id);
CREATE INDEX IF NOT EXISTS idx_accounts_provider_account ON accounts(provider_id, account_id);
"#;

/// Our own `shares` table (separate from better-auth's tables).
const SHARES_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS shares (
    id         TEXT PRIMARY KEY,
    json       TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"#;

/// Connect to Postgres and ensure both schemas exist.
pub async fn connect() -> anyhow::Result<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL 未设置(填 Supabase Session pooler URI)")?;

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&url)
        .await
        .context("连接数据库失败")?;

    // raw_sql runs the multi-statement migration unprepared.
    sqlx::raw_sql(BETTER_AUTH_CORE_SQL).execute(&pool).await.context("建 better-auth 表")?;
    sqlx::raw_sql(SHARES_SQL).execute(&pool).await.context("建 shares 表")?;
    Ok(pool)
}

/// Test pool: returns `None` (test skips) unless `TEST_DATABASE_URL` is set.
#[cfg(test)]
pub async fn test_pool() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await.ok()?;
    sqlx::raw_sql(SHARES_SQL).execute(&pool).await.ok()?;
    Some(pool)
}
