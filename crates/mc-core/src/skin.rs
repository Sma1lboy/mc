//! Minecraft 皮肤 / 披风管理。
//!
//! 直接对接 Minecraft Services profile API,用某个微软账号的 **Minecraft access
//! token** 作 Bearer:
//! - 读取当前 profile(皮肤 + 披风列表,含 ACTIVE/INACTIVE 状态);
//! - 上传新皮肤(本地 PNG,classic/slim 变体,multipart);
//! - 设置 / 隐藏当前披风。
//!
//! 仅微软正版账号有此 API;离线 / 外置账号无皮肤接口,调用方需在上层拦截。

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};

const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const SKINS_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skins";
const ACTIVE_CAPE_URL: &str = "https://api.minecraftservices.com/minecraft/profile/capes/active";

/// profile 里的一条皮肤。`url` 是可直接 `<img>` 的 PNG;`variant` 为 `classic` / `slim`。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SkinInfo {
    pub id: String,
    pub url: String,
    /// 模型变体:`classic`(经典)或 `slim`(纤细)。
    #[serde(default)]
    pub variant: String,
    /// `ACTIVE` / `INACTIVE`。
    #[serde(default)]
    pub state: String,
}

/// profile 里的一条披风。`url` 是 PNG 预览;`alias` 是披风名(如 `Migrator`)。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CapeInfo {
    pub id: String,
    pub url: String,
    #[serde(default)]
    pub alias: String,
    /// `ACTIVE` / `INACTIVE`。
    #[serde(default)]
    pub state: String,
}

/// 当前账号的皮肤 / 披风快照。
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ProfileSkins {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<SkinInfo>,
    #[serde(default)]
    pub capes: Vec<CapeInfo>,
}

#[derive(Serialize)]
struct CapeBody {
    #[serde(rename = "capeId")]
    cape_id: String,
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

/// 把 profile 端点的非成功响应翻译成清晰错误。401/403 多为 token 失效。
async fn into_error(resp: reqwest::Response, action: &str) -> CoreError {
    let status = resp.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return CoreError::Auth(format!(
            "{action}:登录已过期,请重新登录微软账号(HTTP {status})"
        ));
    }
    let body = resp.text().await.unwrap_or_default();
    // profile 端点的业务错误通常带 errorMessage 字段。
    let msg = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.get("errorMessage")
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| body.trim().to_string());
    CoreError::Auth(format!("{action}失败(HTTP {status}):{msg}"))
}

/// 拉取当前 profile 的皮肤 / 披风。
pub async fn fetch_profile(access_token: &str) -> Result<ProfileSkins> {
    let resp = http()
        .get(PROFILE_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(into_error(resp, "获取皮肤资料").await);
    }
    resp.json::<ProfileSkins>()
        .await
        .map_err(|e| CoreError::Auth(format!("解析皮肤资料失败:{e}")))
}

/// 上传一张本地 PNG 作为新皮肤。`variant` 为 `classic` / `slim`。
pub async fn upload_skin(access_token: &str, png_bytes: &[u8], variant: &str) -> Result<ProfileSkins> {
    let variant = match variant {
        "slim" => "slim",
        _ => "classic",
    };
    let part = reqwest::multipart::Part::bytes(png_bytes.to_vec())
        .file_name("skin.png")
        .mime_str("image/png")
        .map_err(|e| CoreError::other(format!("构造皮肤上传表单失败:{e}")))?;
    let form = reqwest::multipart::Form::new()
        .text("variant", variant)
        .part("file", part);

    let resp = http()
        .post(SKINS_URL)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .multipart(form)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(into_error(resp, "上传皮肤").await);
    }
    // 成功响应即更新后的 profile。
    resp.json::<ProfileSkins>()
        .await
        .map_err(|e| CoreError::Auth(format!("解析上传结果失败:{e}")))
}

/// 设置当前披风(`Some(id)`),或隐藏披风(`None`)。返回更新后的 profile。
pub async fn set_cape(access_token: &str, cape_id: Option<&str>) -> Result<ProfileSkins> {
    let client = http();
    let req = match cape_id {
        Some(id) => client
            .put(ACTIVE_CAPE_URL)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .json(&CapeBody { cape_id: id.to_string() }),
        None => client
            .delete(ACTIVE_CAPE_URL)
            .bearer_auth(access_token)
            .header("Accept", "application/json"),
    };
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(into_error(resp, "设置披风").await);
    }
    resp.json::<ProfileSkins>()
        .await
        .map_err(|e| CoreError::Auth(format!("解析披风设置结果失败:{e}")))
}
