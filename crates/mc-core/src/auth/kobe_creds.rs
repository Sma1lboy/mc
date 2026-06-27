//! kobeMC 账号的「记住密码 / 自动登录」凭据存储。
//!
//! kobe 会话只活在进程内存(reqwest cookie jar,见 `server.rs`),不跨重启。要实现记住密码 +
//! 自动登录,把邮箱 / 密码 + auto_login 标记作为一条 JSON blob 存进系统原生 keyring
//! (macOS Keychain / Windows Credential Manager / Linux keyutils),条目键为 `(SERVICE, KEY)`。
//!
//! 设计要点:
//! - **不落明文盘**:这是用户密码,只走 keyring;keyring 不可用(feature 关 / 运行期报错)时
//!   直接放弃持久化(返回 false / None),**绝不**像 token 那样回退写明文文件——免得密码裸奔在磁盘。
//! - **feature 门控**:`keyring` feature(默认开)关掉后整个后端被剔除,所有操作变 no-op。
//! - **测试用内存 store**:`cfg(test)` 下换成进程内共享 `HashMap`,绝不触碰真实钥匙串,可验证
//!   存→取往返。与 [`super::secret`] 同款做法。

use serde::{Deserialize, Serialize};

/// 记住的 kobeMC 登录凭据。`auto_login` 为真时启动会用它自动登录。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, specta::Type)]
pub struct KobeCredentials {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub auto_login: bool,
}

#[cfg(all(feature = "keyring", not(test)))]
mod imp {
    use super::KobeCredentials;

    const SERVICE: &str = "mc-launcher";
    /// keyring 条目键的另一半(固定;每台机器只记一个 kobe 账号的凭据)。
    const KEY: &str = "kobe-credentials";

    pub(crate) fn load() -> Option<KobeCredentials> {
        let entry = keyring::Entry::new(SERVICE, KEY).ok()?;
        match entry.get_password() {
            Ok(blob) => serde_json::from_str(&blob).ok(),
            Err(_) => None,
        }
    }

    pub(crate) fn save(creds: &KobeCredentials) -> bool {
        let entry = match keyring::Entry::new(SERVICE, KEY) {
            Ok(e) => e,
            Err(_) => return false,
        };
        let blob = match serde_json::to_string(creds) {
            Ok(b) => b,
            Err(_) => return false,
        };
        entry.set_password(&blob).is_ok()
    }

    pub(crate) fn clear() {
        if let Ok(entry) = keyring::Entry::new(SERVICE, KEY) {
            let _ = entry.delete_credential();
        }
    }
}

#[cfg(not(feature = "keyring"))]
mod imp {
    use super::KobeCredentials;

    pub(crate) fn load() -> Option<KobeCredentials> {
        None
    }
    pub(crate) fn save(_creds: &KobeCredentials) -> bool {
        false
    }
    pub(crate) fn clear() {}
}

#[cfg(all(feature = "keyring", test))]
mod imp {
    use super::KobeCredentials;
    use std::sync::{Mutex, OnceLock};

    fn store() -> &'static Mutex<Option<KobeCredentials>> {
        static S: OnceLock<Mutex<Option<KobeCredentials>>> = OnceLock::new();
        S.get_or_init(|| Mutex::new(None))
    }

    pub(crate) fn load() -> Option<KobeCredentials> {
        store().lock().unwrap().clone()
    }
    pub(crate) fn save(creds: &KobeCredentials) -> bool {
        *store().lock().unwrap() = Some(creds.clone());
        true
    }
    pub(crate) fn clear() {
        *store().lock().unwrap() = None;
    }
}

/// 读取记住的凭据;无 / 失败返回 `None`。
pub fn load() -> Option<KobeCredentials> {
    imp::load()
}

/// 保存凭据(记住密码 / 自动登录)。keyring 不可用时返回 `false`(不落明文)。
pub fn save(creds: &KobeCredentials) -> bool {
    imp::save(creds)
}

/// 清除记住的凭据(取消记住 / 退出时调用)。
pub fn clear() {
    imp::clear()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_clear_roundtrip() {
        let c = KobeCredentials {
            email: "a@b.com".into(),
            password: "pw".into(),
            auto_login: true,
        };
        assert!(save(&c));
        assert_eq!(load(), Some(c));
        clear();
        assert_eq!(load(), None);
    }
}
