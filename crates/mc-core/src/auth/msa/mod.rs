//! 微软正版登录链路(设备码流)。
//!
//! 微软登录是一串**串行的 token 交换**,这里把每一步做成独立的 async 方法,
//! 任意一步失败都能定位到具体环节并返回清晰的 [`CoreError`]:
//!
//! ```text
//! ① 设备码流拿 Microsoft access_token (device_code_start → poll_token)
//!      ↓
//! ② Xbox Live 认证   user.auth.xboxlive.com/user/authenticate
//!      ↓ (Xbox token + uhs)
//! ③ XSTS 授权        xsts.auth.xboxlive.com/xsts/authorize
//!      ↓ (XSTS token;此处会返回"无 Xbox 账号/地区不支持/未成年"等 XErr 码)
//! ④ 换 Minecraft token  api.minecraftservices.com/authentication/login_with_xbox
//!      ↓
//! ⑤ 拿 Minecraft profile api.minecraftservices.com/minecraft/profile (uuid + name)
//! ```
//!
//! `refresh_token` 用于过期后免浏览器续期(见 [`MsaClient::refresh`])。

use mc_types::AuthSession;
use serde::Deserialize;
use serde_json::{json, Value};

use super::dashify_uuid;
use crate::error::{CoreError, Result};

/// Vanilla launcher 公开的 Azure 应用 client id。设备码流要求该应用允许
/// "public client / native" 流程,vanilla 的这个 id 满足。
const VANILLA_CLIENT_ID: &str = "00000000402b5328";

/// OAuth scope:Xbox 登录 + 离线访问(后者用于换取 refresh_token)。
const SCOPE: &str = "XboxLive.signin offline_access";

const DEVICE_CODE_URL: &str =
    "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// 微软登录客户端。持有一个共享的 [`reqwest::Client`] 与 OAuth client id。
pub struct MsaClient {
    http: reqwest::Client,
    client_id: String,
}

/// 设备码流第一步返回的信息,需要展示给用户。
#[derive(Debug, Clone)]
pub struct DeviceCodeInfo {
    /// 给用户看的短码,在验证页面输入。
    pub user_code: String,
    /// 用户访问的验证地址(通常 https://www.microsoft.com/link)。
    pub verification_uri: String,
    /// 内部轮询用的设备码,**不要展示给用户**。
    pub device_code: String,
    /// 建议的轮询间隔(秒)。
    pub interval: u64,
    /// 设备码有效期(秒),过期后需重新开始。
    pub expires_in: u64,
}

/// 微软 OAuth token(经过设备码/刷新流后得到)。
#[derive(Debug, Clone)]
pub struct MsaToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

impl MsaClient {
    /// 用 vanilla 公共 client id 创建客户端。
    pub fn new() -> Self {
        Self::with_client_id(VANILLA_CLIENT_ID)
    }

    /// 用自定义 Azure client id 创建客户端(测试或自有应用)。
    pub fn with_client_id(client_id: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            client_id: client_id.into(),
        }
    }

    /// 复用一个已有的 [`reqwest::Client`](例如 [`crate::download::Downloader`] 的)。
    pub fn with_http(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    // ── ① 设备码流 ──────────────────────────────────────────────────────

    /// 启动设备码流。返回的 [`DeviceCodeInfo`] 里的 `user_code` /
    /// `verification_uri` 展示给用户,`device_code` 交给 [`Self::poll_token`]。
    pub async fn device_code_start(&self) -> Result<DeviceCodeInfo> {
        // devicecode 端点要求 application/x-www-form-urlencoded。
        let resp = self
            .http
            .post(DEVICE_CODE_URL)
            .form(&[("client_id", self.client_id.as_str()), ("scope", SCOPE)])
            .send()
            .await?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Auth(format!("device code: 响应非 JSON: {e}")))?;

        if !status.is_success() {
            let desc = body
                .get("error_description")
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return Err(CoreError::Auth(format!("device code 请求失败: {desc}")));
        }

        let get_str = |k: &str| -> Result<String> {
            body.get(k)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| CoreError::Auth(format!("device code 响应缺少字段 {k}")))
        };

        Ok(DeviceCodeInfo {
            user_code: get_str("user_code")?,
            verification_uri: get_str("verification_uri")?,
            device_code: get_str("device_code")?,
            // interval/expires_in 可能缺省,给出合理默认值。
            interval: body.get("interval").and_then(Value::as_u64).unwrap_or(5),
            expires_in: body
                .get("expires_in")
                .and_then(Value::as_u64)
                .unwrap_or(900),
        })
    }

    /// 轮询 token 端点直到用户完成授权。
    ///
    /// 处理 `authorization_pending`(继续等)与 `slow_down`(放慢轮询);
    /// `authorization_declined` / `expired_token` / `bad_verification_code`
    /// 等终止错误直接返回 [`CoreError::Auth`]。
    pub async fn poll_token(&self, device_code: &str, interval: u64) -> Result<MsaToken> {
        // 至少 1 秒,避免 0 间隔忙轮询。
        let mut delay = interval.max(1);
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;

            let resp = self
                .http
                .post(TOKEN_URL)
                .form(&[
                    ("client_id", self.client_id.as_str()),
                    ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                    ("device_code", device_code),
                ])
                .send()
                .await?;

            let status = resp.status();
            let body: Value = resp
                .json()
                .await
                .map_err(|e| CoreError::Auth(format!("token 轮询: 响应非 JSON: {e}")))?;

            if status.is_success() {
                return parse_token(&body);
            }

            // 错误响应:根据 error 字段决定继续等待还是终止。
            match body.get("error").and_then(Value::as_str) {
                Some("authorization_pending") => continue,
                Some("slow_down") => {
                    // 微软要求放慢:把间隔加 5 秒。
                    delay += 5;
                    continue;
                }
                Some(other) => {
                    let desc = body
                        .get("error_description")
                        .and_then(Value::as_str)
                        .unwrap_or(other);
                    return Err(CoreError::Auth(format!("设备码授权失败 ({other}): {desc}")));
                }
                None => {
                    return Err(CoreError::Auth(
                        "token 轮询返回未知错误响应".to_string(),
                    ))
                }
            }
        }
    }

    /// 用 `refresh_token` 续期,得到新的 [`MsaToken`]。免浏览器/免设备码。
    pub async fn refresh(&self, refresh_token: &str) -> Result<MsaToken> {
        let resp = self
            .http
            .post(TOKEN_URL)
            .form(&[
                ("client_id", self.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("scope", SCOPE),
            ])
            .send()
            .await?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Auth(format!("refresh: 响应非 JSON: {e}")))?;

        if !status.is_success() {
            let desc = body
                .get("error_description")
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return Err(CoreError::Auth(format!("刷新登录失败: {desc}")));
        }
        parse_token(&body)
    }

    // ── ②→⑤ 完整链路 ──────────────────────────────────────────────────

    /// 从一个 Microsoft access token 走完 Xbox → XSTS → Minecraft → profile,
    /// 产出最终可用于启动的 [`AuthSession`]。
    pub async fn authenticate(&self, ms_access_token: &str) -> Result<AuthSession> {
        // ② Xbox Live 认证。
        let (xbl_token, _uhs1) = self.xbox_authenticate(ms_access_token).await?;
        // ③ XSTS 授权(同时拿到权威的 uhs 与 xuid)。
        let xsts = self.xsts_authorize(&xbl_token).await?;
        // ④ 换 Minecraft access token。
        let mc_token = self
            .minecraft_login(&xsts.uhs, &xsts.token)
            .await?;
        // ⑤ 拿 profile(uuid + name)。
        let (uuid, name) = self.minecraft_profile(&mc_token).await?;

        Ok(AuthSession {
            username: name,
            uuid,
            access_token: mc_token,
            user_type: "msa".to_string(),
            xuid: xsts.xuid,
        })
    }

    /// ② Xbox Live 认证:用 MS token 换 Xbox token + 用户哈希(uhs)。
    async fn xbox_authenticate(&self, ms_access_token: &str) -> Result<(String, String)> {
        let body = json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                // RPS ticket 必须带 "d=" 前缀。
                "RpsTicket": format!("d={ms_access_token}"),
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT",
        });

        let resp = self
            .http
            .post(XBL_AUTH_URL)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let value: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Auth(format!("Xbox 认证: 响应非 JSON: {e}")))?;
        if !status.is_success() {
            return Err(CoreError::Auth(format!(
                "Xbox 认证失败 (HTTP {})",
                status.as_u16()
            )));
        }

        let token = value
            .get("Token")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::Auth("Xbox 认证响应缺少 Token".into()))?
            .to_string();
        let uhs = extract_uhs(&value)
            .ok_or_else(|| CoreError::Auth("Xbox 认证响应缺少 uhs".into()))?;
        Ok((token, uhs))
    }

    /// ③ XSTS 授权:用 Xbox token 换 XSTS token,同时返回 uhs 与 xuid。
    ///
    /// 失败时若返回 `XErr` 错误码,翻译成中文 hint 并以 [`CoreError::Xsts`] 返回。
    async fn xsts_authorize(&self, xbl_token: &str) -> Result<XstsResult> {
        let body = json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [xbl_token],
            },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT",
        });

        let resp = self
            .http
            .post(XSTS_AUTH_URL)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let value: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Auth(format!("XSTS 授权: 响应非 JSON: {e}")))?;

        if !status.is_success() {
            // 401 通常携带 XErr 码,翻译成人话。
            if let Some(code) = value.get("XErr").and_then(Value::as_u64) {
                return Err(CoreError::Xsts {
                    code,
                    hint: xsts_hint(code),
                });
            }
            return Err(CoreError::Auth(format!(
                "XSTS 授权失败 (HTTP {})",
                status.as_u16()
            )));
        }

        let token = value
            .get("Token")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::Auth("XSTS 响应缺少 Token".into()))?
            .to_string();
        let uhs =
            extract_uhs(&value).ok_or_else(|| CoreError::Auth("XSTS 响应缺少 uhs".into()))?;
        // xuid 在 DisplayClaims.xui[0].xid,可能缺省(部分账号不返回)。
        let xuid = value
            .get("DisplayClaims")
            .and_then(|c| c.get("xui"))
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|x| x.get("xid"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        Ok(XstsResult { token, uhs, xuid })
    }

    /// ④ 用 XSTS token + uhs 换 Minecraft access token。
    async fn minecraft_login(&self, uhs: &str, xsts_token: &str) -> Result<String> {
        let body = json!({
            "identityToken": format!("XBL3.0 x={uhs};{xsts_token}"),
        });

        let resp = self
            .http
            .post(MC_LOGIN_URL)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let value: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Auth(format!("Minecraft 登录: 响应非 JSON: {e}")))?;
        if !status.is_success() {
            return Err(CoreError::Auth(format!(
                "Minecraft 登录失败 (HTTP {})",
                status.as_u16()
            )));
        }

        value
            .get("access_token")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| CoreError::Auth("Minecraft 登录响应缺少 access_token".into()))
    }

    /// ⑤ 用 Minecraft access token 拿 profile(uuid + name)。
    ///
    /// 若账号未购买游戏,profile 端点返回 404,这里翻译成清晰提示。
    async fn minecraft_profile(&self, mc_token: &str) -> Result<(String, String)> {
        let resp = self
            .http
            .get(MC_PROFILE_URL)
            .bearer_auth(mc_token)
            .header("Accept", "application/json")
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(CoreError::Auth(
                "该微软账号没有 Minecraft Java 版,请先购买游戏".into(),
            ));
        }

        let value: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Auth(format!("获取 profile: 响应非 JSON: {e}")))?;
        if !status.is_success() {
            // profile 端点的业务错误会带 errorMessage。
            let msg = value
                .get("errorMessage")
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return Err(CoreError::Auth(format!("获取 profile 失败: {msg}")));
        }

        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::Auth("profile 响应缺少 id".into()))?;
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| CoreError::Auth("profile 响应缺少 name".into()))?
            .to_string();
        // Minecraft profile 的 id 是无连字符的 32 位 hex,转成标准带连字符形式。
        Ok((dashify_uuid(id), name))
    }
}

impl Default for MsaClient {
    fn default() -> Self {
        Self::new()
    }
}

/// XSTS 授权的产物。
struct XstsResult {
    token: String,
    uhs: String,
    xuid: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: u64,
}

/// 解析 OAuth token 响应为 [`MsaToken`]。
fn parse_token(body: &Value) -> Result<MsaToken> {
    let parsed: TokenResponse = serde_json::from_value(body.clone())
        .map_err(|e| CoreError::Auth(format!("解析 token 响应失败: {e}")))?;
    if parsed.access_token.is_empty() {
        return Err(CoreError::Auth("token 响应缺少 access_token".into()));
    }
    Ok(MsaToken {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_in: parsed.expires_in,
    })
}

/// 从 Xbox/XSTS 响应的 `DisplayClaims.xui[0].uhs` 提取用户哈希。
fn extract_uhs(value: &Value) -> Option<String> {
    value
        .get("DisplayClaims")
        .and_then(|c| c.get("xui"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|x| x.get("uhs"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// 把已知的 XSTS XErr 码翻译成中文提示。未知码给出通用提示。
fn xsts_hint(code: u64) -> String {
    match code {
        2148916233 => {
            "此微软账号没有关联的 Xbox 账号,请先用该账号登录一次 Xbox 创建档案".to_string()
        }
        2148916235 => "Xbox Live 在当前国家/地区不可用".to_string(),
        2148916236 | 2148916237 => {
            "该账号需要完成成人验证(部分地区如韩国要求)".to_string()
        }
        2148916238 => {
            "该账号属于未成年人,需先由家长将其加入家庭组才能登录".to_string()
        }
        _ => format!("Xbox 授权被拒绝(错误码 {code})"),
    }
}

#[cfg(test)]
mod tests;
