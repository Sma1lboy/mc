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

/// Private realms (临时领域)+ their members. A realm is a code-joined, *non*-
/// discoverable shared mod set: the owner pushes a versioned `manifest` (the
/// file list to sync), friends join by `code`, and the launcher reconciles each
/// member's instance to the manifest (the "外侧 syncer"). `gen_random_uuid()`
/// (present on Supabase / PG13+) generates ids + join codes server-side.
const REALMS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS realms (
    id               TEXT PRIMARY KEY,
    code             TEXT NOT NULL UNIQUE,
    name             TEXT NOT NULL,
    owner_id         TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    mc_version       TEXT,
    loader           TEXT,
    manifest         JSONB NOT NULL DEFAULT '{}'::jsonb,
    manifest_version INTEGER NOT NULL DEFAULT 0,
    expires_at       TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE TABLE IF NOT EXISTS realm_members (
    realm_id       TEXT NOT NULL REFERENCES realms(id) ON DELETE CASCADE,
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role           TEXT NOT NULL DEFAULT 'member',
    synced_version INTEGER NOT NULL DEFAULT 0,
    joined_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (realm_id, user_id)
);
CREATE INDEX IF NOT EXISTS idx_realms_code ON realms(code);
CREATE INDEX IF NOT EXISTS idx_realm_members_user ON realm_members(user_id);
"#;

/// Friendships — one **directed** row per request: `(requester, addressee, status)`.
/// A→B request inserts `(A,B,'pending')`; B accepting flips it to `'accepted'`.
/// Friends = any `accepted` row where the user is on either side; incoming
/// requests = `pending` rows addressed to the user.
const FRIENDS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS friendships (
    requester_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    addressee_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    status       TEXT NOT NULL DEFAULT 'pending',
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (requester_id, addressee_id)
);
CREATE INDEX IF NOT EXISTS idx_friendships_addressee ON friendships(addressee_id);
"#;

/// Presence — friend online status + "what they're playing". Two nullable
/// columns on `users`, added idempotently: `last_seen_at` is bumped to `NOW()`
/// on each heartbeat (online = seen within the last 120s), and `activity` is the
/// free-text current activity (e.g. the running instance name), or null when idle.
const PRESENCE_SQL: &str = r#"
ALTER TABLE users ADD COLUMN IF NOT EXISTS last_seen_at TIMESTAMPTZ;
ALTER TABLE users ADD COLUMN IF NOT EXISTS activity TEXT;
"#;

/// Notifications — a typed, per-user inbox the launcher polls. Each row is one
/// event addressed to `user_id` (`kind` = `friend_request` | `friend_accepted` |
/// `realm_invite`); `actor_id` is who caused it and `realm_id` the realm it
/// concerns (both nullable). `read_at` is null until the user opens the bell.
/// `gen_random_uuid()` (Postgres 13+ / Supabase) mints ids server-side.
const NOTIFICATIONS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS notifications (
  id TEXT PRIMARY KEY DEFAULT gen_random_uuid()::text,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  actor_id TEXT REFERENCES users(id) ON DELETE SET NULL,
  realm_id TEXT,
  read_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS notifications_user_idx ON notifications(user_id, created_at DESC);
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
    sqlx::raw_sql(REALMS_SQL).execute(&pool).await.context("建 realms 表")?;
    sqlx::raw_sql(FRIENDS_SQL).execute(&pool).await.context("建 friendships 表")?;
    sqlx::raw_sql(PRESENCE_SQL).execute(&pool).await.context("加 presence 列")?;
    sqlx::raw_sql(NOTIFICATIONS_SQL).execute(&pool).await.context("建 notifications 表")?;
    Ok(pool)
}

/// Test pool: returns `None` (test skips) unless `TEST_DATABASE_URL` is set.
/// Ensures every table a test might touch (better-auth core for the FK to
/// `users`, plus shares + realms).
#[cfg(test)]
pub async fn test_pool() -> Option<PgPool> {
    let url = std::env::var("TEST_DATABASE_URL").ok()?;
    let pool = PgPoolOptions::new().max_connections(2).connect(&url).await.ok()?;
    sqlx::raw_sql(BETTER_AUTH_CORE_SQL).execute(&pool).await.ok()?;
    sqlx::raw_sql(SHARES_SQL).execute(&pool).await.ok()?;
    sqlx::raw_sql(REALMS_SQL).execute(&pool).await.ok()?;
    sqlx::raw_sql(FRIENDS_SQL).execute(&pool).await.ok()?;
    sqlx::raw_sql(PRESENCE_SQL).execute(&pool).await.ok()?;
    sqlx::raw_sql(NOTIFICATIONS_SQL).execute(&pool).await.ok()?;
    Some(pool)
}
