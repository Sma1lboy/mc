//! 外置登录(Yggdrasil / authlib-injector)。
//!
//! 第三方皮肤站(如 LittleSkin、Blessing Skin 等)实现了 Mojang 早期的
//! **Yggdrasil 认证协议**:用户名 + 密码换 `accessToken`,再配合
//! [authlib-injector] 在启动时以 `-javaagent` 注入,把客户端/服务端的
//! 认证请求重定向到皮肤站的 `authserver`,从而在非正版环境里得到正版同款的
//! 皮肤、披风与登录校验。
//!
//! 本模块只负责**认证**(authenticate / refresh / validate)以及生成启动所需的
//! `-javaagent` 参数;injector jar 的下载与放置由上层负责。
//!
//! 协议端点(均相对 `base`,如 `https://littleskin.cn/api/yggdrasil`):
//!
//! ```text
//! POST {base}/authserver/authenticate   用户名+密码 → accessToken + selectedProfile
//! POST {base}/authserver/refresh        旧 token → 新 token(免密续期)
//! POST {base}/authserver/validate       校验 token 是否仍有效(204 = 有效)
//! ```
//!
//! [authlib-injector]: https://github.com/yushijinhun/authlib-injector

use std::path::Path;

use mc_types::AuthSession;
use serde_json::{json, Value};

use crate::error::{CoreError, Result};

/// authlib-injector 约定的 agent 名,authenticate 请求体里的 `agent.name`。
const AGENT_NAME: &str = "Minecraft";
/// agent 版本,Yggdrasil 协议固定为 1。
const AGENT_VERSION: u32 = 1;

/// 外置登录客户端。持有共享的 [`reqwest::Client`] 与 `authserver` 根地址。
///
/// `base` 是皮肤站给出的 **authlib-injector API 根**,例如
/// `https://littleskin.cn/api/yggdrasil`;各端点在其下拼接
/// `/authserver/authenticate` 等子路径。尾部多余的 `/` 会被规范化掉,
/// 避免出现 `//authserver` 这样的双斜杠。
pub struct YggdrasilClient {
    http: reqwest::Client,
    base: String,
}

/// 一次成功认证(或刷新)后得到的会话。
///
/// `client_token` 必须在 authenticate / refresh / validate 之间保持一致:
/// Yggdrasil 用它来标识"同一台客户端",刷新时若 client_token 不匹配会失败,
/// 因此持久化账号时需要把它一并存下来。
#[derive(Debug, Clone)]
pub struct YggdrasilSession {
    pub access_token: String,
    pub client_token: String,
    pub uuid: String,
    pub username: String,
}

impl YggdrasilSession {
    /// 归一到统一的 [`AuthSession`] 出口,供启动阶段使用。
    ///
    /// `user_type` 取 `"msa"`:配合 authlib-injector 时,客户端走的是正版同款的
    /// 在线校验链路,用 `msa` 比 `legacy` 更贴近真实行为(部分版本对 `legacy`
    /// 会走老的、已下线的会话服务器)。`xuid` 留空(仅微软账号有)。
    pub fn to_auth_session(&self) -> AuthSession {
        AuthSession {
            username: self.username.clone(),
            uuid: self.uuid.clone(),
            access_token: self.access_token.clone(),
            user_type: "msa".to_string(),
            xuid: String::new(),
        }
    }
}

impl YggdrasilClient {
    /// 用给定的 authserver 根地址创建客户端。尾部斜杠会被去掉。
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: normalize_base(&base.into()),
        }
    }

    /// 复用一个已有的 [`reqwest::Client`](例如 [`crate::download::Downloader`] 的)。
    pub fn with_http(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    /// authserver 根地址(已规范化,无尾部斜杠)。
    pub fn base(&self) -> &str {
        &self.base
    }

    /// 用用户名 + 密码登录。
    ///
    /// 对应 `POST {base}/authserver/authenticate`。请求体里携带一个由用户名
    /// 派生的稳定 `clientToken`(见 [`stable_client_token`]),`requestUser`
    /// 设为 `false`(我们不需要完整 user 对象)。
    ///
    /// 成功后从 `selectedProfile` 取 uuid + name;若皮肤站返回多个角色但没有
    /// 默认选中角色(`selectedProfile` 缺失),则退而取 `availableProfiles` 的
    /// 第一个,并给出清晰的中文提示路径(此时仍可登录,只是用了首个角色)。
    pub async fn authenticate(&self, username: &str, password: &str) -> Result<YggdrasilSession> {
        let client_token = stable_client_token(username);
        let body = json!({
            "username": username,
            "password": password,
            "clientToken": client_token,
            "agent": { "name": AGENT_NAME, "version": AGENT_VERSION },
            "requestUser": false,
        });

        let value = self.post_json("authserver/authenticate", &body).await?;
        parse_session(&value, username, &client_token)
    }

    /// 用旧 token 免密续期。
    ///
    /// 对应 `POST {base}/authserver/refresh`。`accessToken` 与 `clientToken`
    /// 必须是同一次 authenticate 配对得到的,否则皮肤站会以
    /// `ForbiddenOperationException` 拒绝。续期后旧 token 立即失效。
    pub async fn refresh(
        &self,
        access_token: &str,
        client_token: &str,
    ) -> Result<YggdrasilSession> {
        let body = json!({
            "accessToken": access_token,
            "clientToken": client_token,
            "requestUser": false,
        });

        let value = self.post_json("authserver/refresh", &body).await?;
        // refresh 响应不一定回带 selectedProfile 的 name,用空串占位时
        // parse_session 会从响应里尽量取;取不到则保留原 client_token 并报错。
        parse_session(&value, "", client_token)
    }

    /// 校验 token 是否仍然有效。
    ///
    /// 对应 `POST {base}/authserver/validate`。协议约定:**204 No Content**
    /// 表示有效;**403** 表示已失效(此时应走 [`Self::refresh`])。其它非预期
    /// 状态码当作错误返回。
    pub async fn validate(&self, access_token: &str, client_token: &str) -> Result<bool> {
        let url = self.endpoint("authserver/validate");
        let body = json!({
            "accessToken": access_token,
            "clientToken": client_token,
        });

        let resp = self
            .http
            .post(&url)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        match status.as_u16() {
            // 204:token 有效。
            204 => Ok(true),
            // 403:token 失效(协议规定的"无效"分支,不是异常)。
            403 => Ok(false),
            // 其它状态码:尽量解析 errorMessage 给出人话提示。
            _ => {
                let value: Value = resp.json().await.unwrap_or(Value::Null);
                Err(yggdrasil_error(status.as_u16(), &value, "校验登录态"))
            }
        }
    }

    // ── 内部辅助 ────────────────────────────────────────────────────────

    /// 把相对路径拼到 `base` 上,得到完整端点 URL。
    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base, path.trim_start_matches('/'))
    }

    /// POST 一个 JSON body 并解析 JSON 响应;非 2xx 统一翻译成中文错误。
    async fn post_json(&self, path: &str, body: &Value) -> Result<Value> {
        let url = self.endpoint(path);
        let resp = self
            .http
            .post(&url)
            .header("Accept", "application/json")
            .json(body)
            .send()
            .await?;

        let status = resp.status();
        // 即便出错,Yggdrasil 也会回一个 JSON 错误体;但少数代理/网关可能回非
        // JSON,这里宽容处理:解析失败时退化为 Null,由 yggdrasil_error 兜底。
        let value: Value = if status == reqwest::StatusCode::NO_CONTENT {
            Value::Null
        } else {
            resp.json().await.unwrap_or(Value::Null)
        };

        if !status.is_success() {
            return Err(yggdrasil_error(status.as_u16(), &value, "外置登录"));
        }
        Ok(value)
    }

    /// 为指定的 authlib-injector jar 与 `base` 生成 `-javaagent` JVM 参数。
    ///
    /// 见模块级关联函数 [`javaagent_arg`];此处提供一个绑定到当前 `base` 的
    /// 便捷封装,免去调用方再手动传 `base`。
    pub fn javaagent_arg(&self, authlib_injector_path: &Path) -> String {
        javaagent_arg(authlib_injector_path, &self.base)
    }
}

/// 生成外置登录启动所需的 `-javaagent` JVM 参数。
///
/// 形如 `-javaagent:/path/to/authlib-injector.jar=https://littleskin.cn/api/yggdrasil`。
/// authlib-injector 会读取 `=` 后面的 API 根地址,把游戏内所有认证/会话/皮肤
/// 请求重定向到该皮肤站。**必须**作为 JVM 参数(在 `-jar`/主类之前)注入,
/// 否则外置登录不会生效。
///
/// 路径用 [`Path::display`] 直接拼接:Windows 上的反斜杠与空格 authlib-injector
/// 本身能正确处理(JVM 把整个 `-javaagent:...` 当作一个 token),调用方在把它
/// 放进命令行/进程参数数组时应作为单个 argv 元素传递,无需再加引号。
pub fn javaagent_arg(authlib_injector_path: &Path, base: &str) -> String {
    format!(
        "-javaagent:{}={}",
        authlib_injector_path.display(),
        normalize_base(base),
    )
}

/// 由用户名派生一个**稳定**的 clientToken(无连字符 32 位 hex)。
///
/// Yggdrasil 要求同一客户端在 authenticate / refresh 之间使用一致的
/// clientToken。为避免引入 `uuid` / `rand` 依赖,这里用已有的 `md5` 对
/// `"mc-core-yggdrasil:" + username` 取哈希,得到稳定且与离线 UUID 不撞的
/// 32 位 hex 字符串。同一用户名永远得到同一个 token(刷新可复现)。
pub fn stable_client_token(username: &str) -> String {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(b"mc-core-yggdrasil:");
    hasher.update(username.as_bytes());
    let bytes: [u8; 16] = hasher.finalize().into();
    hex::encode(bytes)
}

/// 去掉 `base` 尾部多余的 `/`(可能有多个),保留协议中的 `://`。
fn normalize_base(base: &str) -> String {
    base.trim_end_matches('/').to_string()
}

/// 把皮肤站返回的 JSON 解析成 [`YggdrasilSession`]。
///
/// `fallback_username` 用于 refresh 这类响应里可能不带角色名的场景:优先用
/// 响应里的 `selectedProfile.name`,取不到再退回传入值。`client_token` 取
/// 响应回带的(authenticate 一定回带),否则用传入的请求值兜底。
fn parse_session(
    value: &Value,
    fallback_username: &str,
    request_client_token: &str,
) -> Result<YggdrasilSession> {
    let access_token = value
        .get("accessToken")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CoreError::Auth("外置登录响应缺少 accessToken".into()))?;

    // clientToken:响应回带优先,否则用请求时的值(协议保证两者相等)。
    let client_token = value
        .get("clientToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(request_client_token)
        .to_string();

    // 选中角色:优先 selectedProfile;缺失则取 availableProfiles 第一个。
    let profile = value
        .get("selectedProfile")
        .filter(|p| !p.is_null())
        .or_else(|| {
            value
                .get("availableProfiles")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
        })
        .ok_or_else(|| {
            CoreError::Auth(
                "外置登录成功但账号下没有可用角色,请先到皮肤站创建一个游戏角色".into(),
            )
        })?;

    let uuid = profile
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| CoreError::Auth("外置登录角色缺少 id(uuid)".into()))?;

    let username = profile
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback_username.to_string());

    if username.is_empty() {
        return Err(CoreError::Auth("外置登录角色缺少 name(用户名)".into()));
    }

    Ok(YggdrasilSession {
        access_token,
        client_token,
        // 皮肤站的 id 多为无连字符 32 位 hex,转成标准带连字符形式,
        // 与启动阶段对 uuid 的预期(${auth_uuid})一致。
        uuid: dashify_uuid(&uuid),
        username,
    })
}

/// 把 Yggdrasil 的错误响应翻译成 [`CoreError::Auth`](中文)。
///
/// 协议错误体形如:
/// ```json
/// { "error": "ForbiddenOperationException",
///   "errorMessage": "Invalid credentials. Invalid username or password." }
/// ```
/// 优先用 `errorMessage`;对常见的几类 `error` 给出更友好的中文前缀。
fn yggdrasil_error(status: u16, value: &Value, ctx: &str) -> CoreError {
    let error = value.get("error").and_then(Value::as_str).unwrap_or("");
    let message = value
        .get("errorMessage")
        .and_then(Value::as_str)
        .unwrap_or("");

    // 针对最常见的几类错误给出贴心提示;其余原样透传 errorMessage。
    let hint = match error {
        "ForbiddenOperationException" => {
            if message.to_lowercase().contains("invalid") {
                "用户名或密码错误,或 token 已失效".to_string()
            } else {
                "操作被拒绝".to_string()
            }
        }
        "IllegalArgumentException" => "请求参数不合法(请检查皮肤站地址是否正确)".to_string(),
        "" if message.is_empty() => format!("{ctx}失败 (HTTP {status})"),
        _ => String::new(),
    };

    // 组装最终消息:优先 errorMessage,辅以 hint。
    let detail = if !message.is_empty() && !hint.is_empty() {
        format!("{hint}({message})")
    } else if !message.is_empty() {
        message.to_string()
    } else if !hint.is_empty() {
        hint
    } else {
        format!("{ctx}失败 (HTTP {status})")
    };

    CoreError::Auth(format!("{ctx}失败: {detail}"))
}

/// 将 32 位无连字符 hex UUID 转为标准 8-4-4-4-12 形式;
/// 已带连字符或长度异常则原样返回。
fn dashify_uuid(raw: &str) -> String {
    if raw.contains('-') || raw.len() != 32 {
        return raw.to_string();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &raw[0..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..32],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn javaagent_arg_concatenates_path_and_base() {
        let path = PathBuf::from("/opt/mc/authlib-injector.jar");
        let arg = javaagent_arg(&path, "https://littleskin.cn/api/yggdrasil");
        assert_eq!(
            arg,
            "-javaagent:/opt/mc/authlib-injector.jar=https://littleskin.cn/api/yggdrasil"
        );
    }

    #[test]
    fn javaagent_arg_trims_trailing_slash_on_base() {
        let path = PathBuf::from("/a/b.jar");
        let arg = javaagent_arg(&path, "https://example.com/api/yggdrasil///");
        assert_eq!(arg, "-javaagent:/a/b.jar=https://example.com/api/yggdrasil");
    }

    #[test]
    fn client_javaagent_arg_uses_normalized_base() {
        let client = YggdrasilClient::new("https://littleskin.cn/api/yggdrasil/");
        let arg = client.javaagent_arg(Path::new("/x/injector.jar"));
        assert_eq!(
            arg,
            "-javaagent:/x/injector.jar=https://littleskin.cn/api/yggdrasil"
        );
    }

    #[test]
    fn new_normalizes_base() {
        let c = YggdrasilClient::new("https://host/api/yggdrasil/");
        assert_eq!(c.base(), "https://host/api/yggdrasil");
        let c2 = YggdrasilClient::new("https://host/api/yggdrasil");
        assert_eq!(c2.base(), "https://host/api/yggdrasil");
    }

    #[test]
    fn endpoint_joins_without_double_slash() {
        let c = YggdrasilClient::new("https://host/api/yggdrasil/");
        assert_eq!(
            c.endpoint("authserver/authenticate"),
            "https://host/api/yggdrasil/authserver/authenticate"
        );
        // 即使路径带前导斜杠也不会出现双斜杠。
        assert_eq!(
            c.endpoint("/authserver/validate"),
            "https://host/api/yggdrasil/authserver/validate"
        );
    }

    #[test]
    fn stable_client_token_is_deterministic_and_hex() {
        let a = stable_client_token("alice");
        let b = stable_client_token("alice");
        assert_eq!(a, b, "同一用户名必须得到相同 clientToken");
        assert_eq!(a.len(), 32, "应为 32 位 hex");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // 不同用户名应不同。
        assert_ne!(stable_client_token("alice"), stable_client_token("bob"));
    }

    #[test]
    fn parse_session_reads_selected_profile() {
        // 模拟 authenticate 成功响应。
        let v = json!({
            "accessToken": "AT",
            "clientToken": "CT",
            "selectedProfile": {
                "id": "4566e69fc90748ee8a1015c8b41d1c00",
                "name": "Steve"
            },
            "availableProfiles": [
                { "id": "4566e69fc90748ee8a1015c8b41d1c00", "name": "Steve" }
            ]
        });
        let s = parse_session(&v, "ignored", "REQ_CT").unwrap();
        assert_eq!(s.access_token, "AT");
        // 响应回带 clientToken 时优先用它。
        assert_eq!(s.client_token, "CT");
        assert_eq!(s.username, "Steve");
        // id 被转成带连字符形式。
        assert_eq!(s.uuid, "4566e69f-c907-48ee-8a10-15c8b41d1c00");
    }

    #[test]
    fn parse_session_falls_back_to_available_profiles() {
        // 没有 selectedProfile 时取 availableProfiles 第一个。
        let v = json!({
            "accessToken": "AT",
            "availableProfiles": [
                { "id": "00000000000000000000000000000001", "name": "First" },
                { "id": "00000000000000000000000000000002", "name": "Second" }
            ]
        });
        let s = parse_session(&v, "fallback", "REQ_CT").unwrap();
        assert_eq!(s.username, "First");
        // 响应没回带 clientToken 时用请求值兜底。
        assert_eq!(s.client_token, "REQ_CT");
    }

    #[test]
    fn parse_session_uses_request_client_token_when_absent() {
        let v = json!({
            "accessToken": "AT",
            "selectedProfile": { "id": "ab", "name": "X" }
        });
        let s = parse_session(&v, "", "REQ_CT").unwrap();
        assert_eq!(s.client_token, "REQ_CT");
        // 非 32 位 id 原样保留。
        assert_eq!(s.uuid, "ab");
    }

    #[test]
    fn parse_session_errors_on_missing_access_token() {
        let v = json!({ "selectedProfile": { "id": "ab", "name": "X" } });
        assert!(parse_session(&v, "", "CT").is_err());
    }

    #[test]
    fn parse_session_errors_when_no_profile() {
        let v = json!({ "accessToken": "AT", "clientToken": "CT" });
        let err = parse_session(&v, "", "CT").unwrap_err();
        match err {
            CoreError::Auth(m) => assert!(m.contains("角色"), "应提示无可用角色: {m}"),
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn parse_session_uses_fallback_username_when_profile_name_empty() {
        let v = json!({
            "accessToken": "AT",
            "clientToken": "CT",
            "selectedProfile": { "id": "ab", "name": "" }
        });
        let s = parse_session(&v, "FallbackName", "CT").unwrap();
        assert_eq!(s.username, "FallbackName");
    }

    #[test]
    fn yggdrasil_error_parses_error_message() {
        // 典型的凭据错误响应。
        let v = json!({
            "error": "ForbiddenOperationException",
            "errorMessage": "Invalid credentials. Invalid username or password."
        });
        let err = yggdrasil_error(403, &v, "外置登录");
        match err {
            CoreError::Auth(m) => {
                assert!(m.contains("外置登录失败"), "前缀缺失: {m}");
                assert!(m.contains("用户名或密码错误"), "应有中文提示: {m}");
                assert!(m.contains("Invalid credentials"), "应透传原始消息: {m}");
            }
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn yggdrasil_error_passes_through_unknown_message() {
        let v = json!({
            "error": "SomeOtherException",
            "errorMessage": "皮肤站维护中"
        });
        let err = yggdrasil_error(500, &v, "校验登录态");
        match err {
            CoreError::Auth(m) => {
                assert!(m.contains("校验登录态失败"));
                assert!(m.contains("皮肤站维护中"));
            }
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn yggdrasil_error_falls_back_to_status_when_empty() {
        // 完全没有 JSON 错误体(代理回了空)。
        let err = yggdrasil_error(502, &Value::Null, "外置登录");
        match err {
            CoreError::Auth(m) => {
                assert!(m.contains("502"), "应带状态码: {m}");
                assert!(m.contains("外置登录失败"));
            }
            other => panic!("期望 Auth 错误,得到 {other:?}"),
        }
    }

    #[test]
    fn to_auth_session_maps_fields() {
        let sess = YggdrasilSession {
            access_token: "AT".into(),
            client_token: "CT".into(),
            uuid: "4566e69f-c907-48ee-8a10-15c8b41d1c00".into(),
            username: "Steve".into(),
        };
        let a = sess.to_auth_session();
        assert_eq!(a.username, "Steve");
        assert_eq!(a.uuid, "4566e69f-c907-48ee-8a10-15c8b41d1c00");
        assert_eq!(a.access_token, "AT");
        // 外置登录归一为 msa,xuid 空。
        assert_eq!(a.user_type, "msa");
        assert!(a.xuid.is_empty());
    }

    #[test]
    fn dashify_roundtrip() {
        assert_eq!(
            dashify_uuid("4566e69fc90748ee8a1015c8b41d1c00"),
            "4566e69f-c907-48ee-8a10-15c8b41d1c00"
        );
        // 已带连字符:原样。
        let dashed = "4566e69f-c907-48ee-8a10-15c8b41d1c00";
        assert_eq!(dashify_uuid(dashed), dashed);
        // 长度异常:原样。
        assert_eq!(dashify_uuid("abc"), "abc");
    }
}
