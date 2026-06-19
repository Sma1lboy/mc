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
pub use store::{AccountStore, StoredAccount};
pub use yggdrasil::{YggdrasilClient, YggdrasilSession};
