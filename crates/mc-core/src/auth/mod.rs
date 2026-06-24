//! 账号认证。
//!
//! 三种账号类型,全部归一到 [`mc_types::AuthSession`] 这一个出口,启动阶段对
//! 账号类型无感知:
//!
//! - [`offline`] — 离线账号:用户名 → 稳定 UUID,无网络验证。
//! - [`msa`] — 微软正版:设备码流登录,串行 token 交换链。
//! - [`store`] — 多账号持久化(JSON),导出统一的 [`AuthSession`]。
//!
//! 外置登录(Yggdrasil / authlib-injector)与离线/微软共用同一个
//! [`store::AccountStore`] 出口;其认证客户端可后续在 `yggdrasil` 子模块补充。

pub mod msa;
pub mod offline;
pub mod store;
pub mod yggdrasil;

pub use msa::{DeviceCodeInfo, MsaClient, MsaToken};
pub use offline::offline_session;
pub use store::{now_unix, AccountStore, StoredAccount};
pub use yggdrasil::{YggdrasilClient, YggdrasilSession};

use crate::error::Result;
use mc_types::AccountKind;

/// Minecraft access token 的典型有效期(约 24 小时)。续期后据此重置 `expires_at`。
pub const MC_TOKEN_TTL_SECS: i64 = 86_400;

/// 若当前选中的是(接近)过期的微软账号且有 refresh_token,就用它免浏览器续期,
/// 刷新后的账号写回 `store` 并保持选中。返回是否真的执行了续期。
///
/// 非微软 / 仍新鲜 / 无 refresh_token 时直接返回 `Ok(false)`(no-op)。续期失败
/// (refresh_token 失效、网络故障)以 `Err` 上抛:启动路径可best-effort 忽略并用旧
/// token 继续,显式「刷新登录」入口则把错误展示给用户提示重新登录。
pub async fn refresh_selected_microsoft(
    store: &mut AccountStore,
    msa: &MsaClient,
    margin_secs: i64,
) -> Result<bool> {
    let (refresh_token, owns_game) = match store.selected_account() {
        Some(a)
            if a.kind == AccountKind::Microsoft
                && a.is_expired(now_unix(), margin_secs)
                && a.refresh_token.is_some() =>
        {
            (a.refresh_token.clone().unwrap(), a.owns_game)
        }
        _ => return Ok(false),
    };

    let token = msa.refresh(&refresh_token).await?;
    let session = msa.authenticate(&token.access_token).await?;

    let updated = StoredAccount {
        kind: AccountKind::Microsoft,
        username: session.username.clone(),
        uuid: session.uuid.clone(),
        access_token: session.access_token.clone(),
        // 刷新端点可能不返回新的 refresh_token,这种情况下继续沿用旧的。
        refresh_token: Some(if token.refresh_token.is_empty() {
            refresh_token
        } else {
            token.refresh_token
        }),
        xuid: session.xuid.clone(),
        user_type: session.user_type.clone(),
        owns_game,
        expires_at: Some(now_unix() + MC_TOKEN_TTL_SECS),
        client_token: None,
        yggdrasil_base: None,
    };

    let uuid = updated.uuid.clone();
    store.add(updated); // 按 uuid 原地更新
    let _ = store.select(&uuid); // 保持选中
    store.save()?;
    Ok(true)
}

/// 若当前选中的是外置登录(Yggdrasil)账号,启动前校验其 access_token;失效则用持久化的
/// `client_token` 免密续期并写回 `store`(保持选中)。返回是否真的执行了续期。
///
/// 非外置 / 缺少 `client_token` 或 `yggdrasil_base`(老数据)时直接返回 `Ok(false)`(no-op)。
/// 校验或续期的网络/协议错误以 `Err` 上抛:启动路径可 best-effort 忽略(用现有 token 继续),
/// 显式「刷新登录」入口则据此提示用户重新登录。续期不改变账号身份(uuid/username/base 沿用),
/// 只替换 access/client token —— 与微软续期一致的「原地更新」语义。
pub async fn refresh_selected_yggdrasil(
    store: &mut AccountStore,
    http: reqwest::Client,
) -> Result<bool> {
    let acc = match store.selected_account() {
        Some(a) if a.kind == AccountKind::Yggdrasil => a.clone(),
        _ => return Ok(false),
    };
    let (base, client_token) = match (acc.yggdrasil_base.clone(), acc.client_token.clone()) {
        (Some(b), Some(c)) => (b, c),
        _ => return Ok(false),
    };

    let client = YggdrasilClient::new(base).with_http(http);
    match client.refresh_session(&acc.access_token, &client_token).await? {
        // token 仍有效:无需续期。
        None => Ok(false),
        // 续期成功:仅更新 token,其余身份字段沿用原账号。
        Some(session) => {
            let updated = StoredAccount {
                access_token: session.access_token,
                client_token: Some(session.client_token),
                ..acc
            };
            let uuid = updated.uuid.clone();
            store.add(updated);
            let _ = store.select(&uuid);
            store.save()?;
            Ok(true)
        }
    }
}
