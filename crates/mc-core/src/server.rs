//! Client for our own lite backend (`mc-server`). The launcher reaches loader
//! aggregation, news and instance sharing through one base URL.
//!
//! The base URL comes from `MC_SERVER_URL` (set it to `http://127.0.0.1:8787`
//! for the local test server) and otherwise falls back to the production host.

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

// TEMP DEV default: the current Railway dev deployment of mc-server. This URL may
// be rotated/refreshed — override with `MC_SERVER_URL`, and swap to the stable
// production host before GA. Marked temporary on purpose.
const DEFAULT_BASE: &str = "https://mc-server-production-9152.up.railway.app";

#[derive(Debug, Clone, Deserialize)]
pub struct LoaderMeta {
    pub mc_version: String,
    pub loaders: Loaders,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Loaders {
    #[serde(default)]
    pub fabric: Vec<String>,
    #[serde(default)]
    pub quilt: Vec<String>,
    #[serde(default)]
    pub forge: Vec<String>,
    #[serde(default)]
    pub neoforge: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct NewsItem {
    pub id: String,
    pub title: String,
    pub body: String,
    pub date: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedFile {
    pub path: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedInstance {
    pub name: String,
    pub mc_version: String,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub files: Vec<SharedFile>,
    #[serde(default)]
    pub id: String,
}

/// The authenticated user (better-auth user shape). The session lives in the
/// reqwest cookie jar (better-auth sets a session cookie), so keep one
/// `ServerClient` and its session persists across calls.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthUser {
    pub id: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
}

/// `sign-up`/`sign-in` return `{ token, user }` — we use the session cookie, so
/// only `user` is read (the `token` field is ignored by serde).
#[derive(Debug, Clone, Deserialize)]
struct AuthResponse {
    user: AuthUser,
}

/// `get-session` returns `{ session, user }`.
#[derive(Debug, Clone, Deserialize)]
struct SessionResponse {
    user: AuthUser,
}

pub struct ServerClient {
    base: String,
    http: reqwest::Client,
}

impl ServerClient {
    /// Build a client honoring `MC_SERVER_URL`, else the production default.
    pub fn new() -> Result<Self> {
        let base = std::env::var("MC_SERVER_URL").unwrap_or_else(|_| DEFAULT_BASE.to_string());
        Self::with_base(base)
    }

    pub fn with_base(base: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(format!("mc-launcher/{}", crate::LAUNCHER_VERSION))
            // Persist the auth session cookie across requests on this client.
            .cookie_store(true)
            .build()?;
        Ok(Self { base: base.into().trim_end_matches('/').to_string(), http })
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(CoreError::other(format!("server {} returned {}", path, resp.status())));
        }
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| CoreError::Parse { what: format!("server {path}"), source: e })
    }

    /// Liveness check; returns the raw status json.
    pub async fn health(&self) -> Result<serde_json::Value> {
        self.get_json("/v1/health").await
    }

    /// Aggregated loader versions for a Minecraft version.
    pub async fn loaders(&self, mc_version: &str) -> Result<LoaderMeta> {
        self.get_json(&format!("/v1/meta/loaders/{mc_version}")).await
    }

    pub async fn news(&self) -> Result<Vec<NewsItem>> {
        self.get_json("/v1/news").await
    }

    /// Publish an instance for sharing; returns its short id.
    pub async fn share_instance(&self, inst: &SharedInstance) -> Result<String> {
        let url = format!("{}/v1/instances/share", self.base);
        let resp = self.http.post(&url).json(inst).send().await?;
        let v: serde_json::Value = resp.json().await?;
        v.get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CoreError::other("share response missing id"))
    }

    /// Fetch a shared instance by id.
    pub async fn get_instance(&self, id: &str) -> Result<SharedInstance> {
        self.get_json(&format!("/v1/instances/{id}")).await
    }

    /// Register a launcher account (better-auth email sign-up). The session
    /// cookie is stored on this client, so subsequent calls (e.g. [`me`]) are
    /// authenticated.
    pub async fn register(&self, email: &str, password: &str, name: &str) -> Result<AuthUser> {
        let body = serde_json::json!({ "email": email, "password": password, "name": name });
        let r: AuthResponse = self.auth_post("/v1/auth/sign-up/email", body).await?;
        Ok(r.user)
    }

    /// Log in (better-auth email sign-in); establishes the session cookie.
    pub async fn login(&self, email: &str, password: &str) -> Result<AuthUser> {
        let body = serde_json::json!({ "email": email, "password": password });
        let r: AuthResponse = self.auth_post("/v1/auth/sign-in/email", body).await?;
        Ok(r.user)
    }

    async fn auth_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(CoreError::Auth(format!("{} 返回 {}", path, resp.status())));
        }
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| CoreError::Parse { what: format!("server {path}"), source: e })
    }

    /// The current session's user (better-auth `get-session`, uses the cookie).
    /// Errors if not logged in / the session expired.
    pub async fn me(&self) -> Result<AuthUser> {
        let url = format!("{}/v1/auth/get-session", self.base);
        let resp = self.http.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(CoreError::Auth(format!("会话无效({})", resp.status())));
        }
        let bytes = resp.bytes().await?;
        let s: SessionResponse = serde_json::from_slice(&bytes)
            .map_err(|e| CoreError::Parse { what: "server get-session".into(), source: e })?;
        Ok(s.user)
    }

    /// Log out — clears the better-auth server-side session.
    pub async fn logout(&self) -> Result<()> {
        let url = format!("{}/v1/auth/sign-out", self.base);
        self.http.post(&url).send().await?;
        Ok(())
    }
}
