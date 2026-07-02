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

/// Public marketing domain that hosts the read-only share page (a static page
/// that fetches the shared conversation JSON from the API and renders it). The
/// share button hands out `{SHARE_PAGE_BASE}/share/{id}`.
const SHARE_PAGE_BASE: &str = "https://kobemc.sma1lboy.me";

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
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
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

    /// 本客户端拼绝对 URL 的唯一处(原先散在每个动词里 `format!("{}{}", base, path)`)。
    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    /// 非 2xx 状态的统一映射(标准接口错误策略的单一来源);返回原 `Response` 供继续读取。
    fn checked(&self, resp: reqwest::Response, path: &str) -> Result<reqwest::Response> {
        if !resp.status().is_success() {
            return Err(CoreError::other(format!("server {} returned {}", path, resp.status())));
        }
        Ok(resp)
    }

    /// 发送一个已构造好的请求并校验状态。所有标准动词的「发送 + 校验」都收敛到这里——
    /// 像旧 `share_instance` 那样漏掉状态检查、把 4xx body 当成功体解析,在结构上不再可能。
    async fn send_checked(
        &self,
        req: reqwest::RequestBuilder,
        path: &str,
    ) -> Result<reqwest::Response> {
        self.checked(req.send().await?, path)
    }

    /// 读取并反序列化 JSON 响应体(解析错误信息的单一来源)。
    async fn parse_json<T: serde::de::DeserializeOwned>(
        &self,
        resp: reqwest::Response,
        path: &str,
    ) -> Result<T> {
        let bytes = resp.bytes().await?;
        serde_json::from_slice(&bytes)
            .map_err(|e| CoreError::Parse { what: format!("server {path}"), source: e })
    }

    pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.send_checked(self.http.get(self.url(path)), path).await?;
        self.parse_json(resp, path).await
    }

    /// GET with query params and parse the JSON response; errors on non-2xx.
    pub(crate) async fn get_json_query<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let resp = self.send_checked(self.http.get(self.url(path)).query(query), path).await?;
        self.parse_json(resp, path).await
    }

    /// POST a JSON body and parse the JSON response; errors on non-2xx.
    pub(crate) async fn post_json<B: serde::Serialize + ?Sized, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self.send_checked(self.http.post(self.url(path)).json(body), path).await?;
        self.parse_json(resp, path).await
    }

    /// POST expecting a JSON body, but treat **404** as a clean `None`
    /// (used where the resource may legitimately not exist, e.g. a bad join code).
    pub(crate) async fn post_optional_json<
        B: serde::Serialize + ?Sized,
        T: serde::de::DeserializeOwned,
    >(
        &self,
        path: &str,
        body: &B,
    ) -> Result<Option<T>> {
        // 404 在 `checked` 之前判:这条路径把「资源不存在」当成干净的 None,不是错误。
        let resp = self.http.post(self.url(path)).json(body).send().await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let resp = self.checked(resp, path)?;
        self.parse_json(resp, path).await.map(Some)
    }

    /// POST a JSON body, discarding the (empty) response; errors on non-2xx.
    pub(crate) async fn post_no_content<B: serde::Serialize + ?Sized>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<()> {
        self.send_checked(self.http.post(self.url(path)).json(body), path).await?;
        Ok(())
    }

    /// DELETE a resource, discarding the (empty) response; errors on non-2xx.
    pub(crate) async fn delete_no_content(&self, path: &str) -> Result<()> {
        self.send_checked(self.http.delete(self.url(path)), path).await?;
        Ok(())
    }

    /// POST a raw binary body (e.g. the realm overrides zip); errors on non-2xx.
    pub(crate) async fn post_bytes(&self, path: &str, body: Vec<u8>) -> Result<()> {
        let req = self
            .http
            .post(self.url(path))
            .header(reqwest::header::CONTENT_TYPE, "application/zip")
            .body(body);
        self.send_checked(req, path).await?;
        Ok(())
    }

    /// GET a raw binary body (e.g. the realm overrides zip); errors on non-2xx.
    pub(crate) async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let resp = self.send_checked(self.http.get(self.url(path)), path).await?;
        Ok(resp.bytes().await?.to_vec())
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
        let path = "/v1/instances/share";
        let resp = self.send_checked(self.http.post(self.url(path)).json(inst), path).await?;
        let v: serde_json::Value = self.parse_json(resp, path).await?;
        v.get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CoreError::other("share response missing id"))
    }

    /// Fetch a shared instance by id.
    pub async fn get_instance(&self, id: &str) -> Result<SharedInstance> {
        self.get_json(&format!("/v1/instances/{id}")).await
    }

    /// Publish an agent chat transcript (opaque JSON) for public sharing; returns
    /// `(id, public_url)`.
    pub async fn share_conversation(&self, payload: &serde_json::Value) -> Result<(String, String)> {
        let path = "/v1/agent/conversations";
        let resp = self.send_checked(self.http.post(self.url(path)).json(payload), path).await?;
        let v: serde_json::Value = self.parse_json(resp, path).await?;
        let id = v
            .get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CoreError::other("share response missing id"))?;
        // Hand out the human-facing share page (the landing frontend renders the
        // conversation by fetching this API), not the raw JSON endpoint. The page
        // lives on the marketing domain, independent of the API base.
        let url = format!("{SHARE_PAGE_BASE}/share/{id}");
        Ok((id, url))
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

    // auth 动词的非 2xx 故意映射成 CoreError::Auth(中文文案),与标准 `checked` 策略不同,
    // 是有意的领域差异,因此各自保留状态检查;URL 与 JSON 解析仍走共用 owner。
    async fn auth_post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T> {
        let resp = self.http.post(self.url(path)).json(&body).send().await?;
        if !resp.status().is_success() {
            return Err(CoreError::Auth(format!("{} 返回 {}", path, resp.status())));
        }
        self.parse_json(resp, path).await
    }

    /// The current session's user (better-auth `get-session`, uses the cookie).
    /// Errors if not logged in / the session expired.
    pub async fn me(&self) -> Result<AuthUser> {
        let path = "/v1/auth/get-session";
        let resp = self.http.get(self.url(path)).send().await?;
        if !resp.status().is_success() {
            return Err(CoreError::Auth(format!("会话无效({})", resp.status())));
        }
        let s: SessionResponse = self.parse_json(resp, path).await?;
        Ok(s.user)
    }

    /// Log out — clears the better-auth server-side session.
    pub async fn logout(&self) -> Result<()> {
        self.http.post(self.url("/v1/auth/sign-out")).send().await?;
        Ok(())
    }
}
