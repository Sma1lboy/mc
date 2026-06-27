//! kobeMC 账号的「记住密码 / 自动登录」凭据存储 —— 一个**列表**(可记多个账号)。
//!
//! kobe 会话只活在进程内存(reqwest cookie jar,见 `server.rs`),不跨重启。要实现记住密码 +
//! 自动登录,把一组 {email, password, auto_login} 作为一条 JSON 数组存进系统原生 keyring
//! (macOS Keychain / Windows Credential Manager / Linux keyutils),条目键为 `(SERVICE, KEY)`。
//! 用户可记多个账号,从列表里挑一个登录;至多一个标记为自动登录(启动时用它静默登录)。
//!
//! 设计要点:
//! - **不落明文盘**:这是用户密码,只走 keyring;keyring 不可用(feature 关 / 运行期报错)时
//!   直接放弃持久化(返回空 / no-op),**绝不**回退写明文文件——免得密码裸奔在磁盘。
//! - **按 email 去重**:同邮箱再存即覆盖(更新密码 / 自动登录标记)。
//! - **自动登录唯一**:把某账号设为自动登录时,其余账号的该标记一律清掉。
//! - **feature 门控 + 测试用内存 store**:与 [`super::secret`] 同款做法,测试绝不触碰真实钥匙串。

use serde::{Deserialize, Serialize};

/// 记住的一个 kobeMC 账号凭据。`auto_login` 为真的(至多一个)启动时自动登录。
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
    /// keyring 条目键的另一半(固定;整张账号列表存这一条目下)。
    const KEY: &str = "kobe-credentials";

    pub(crate) fn read() -> Vec<KobeCredentials> {
        let Ok(entry) = keyring::Entry::new(SERVICE, KEY) else {
            return Vec::new();
        };
        match entry.get_password() {
            Ok(blob) => serde_json::from_str(&blob).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    pub(crate) fn write(list: &[KobeCredentials]) -> bool {
        let entry = match keyring::Entry::new(SERVICE, KEY) {
            Ok(e) => e,
            Err(_) => return false,
        };
        // 空列表 → 删条目(别留一个 "[]" 的空壳)。
        if list.is_empty() {
            let _ = entry.delete_credential();
            return true;
        }
        let blob = match serde_json::to_string(list) {
            Ok(b) => b,
            Err(_) => return false,
        };
        entry.set_password(&blob).is_ok()
    }
}

#[cfg(not(feature = "keyring"))]
mod imp {
    use super::KobeCredentials;

    pub(crate) fn read() -> Vec<KobeCredentials> {
        Vec::new()
    }
    pub(crate) fn write(_list: &[KobeCredentials]) -> bool {
        false
    }
}

#[cfg(all(feature = "keyring", test))]
mod imp {
    use super::KobeCredentials;
    use std::sync::{Mutex, OnceLock};

    fn store() -> &'static Mutex<Vec<KobeCredentials>> {
        static S: OnceLock<Mutex<Vec<KobeCredentials>>> = OnceLock::new();
        S.get_or_init(|| Mutex::new(Vec::new()))
    }

    pub(crate) fn read() -> Vec<KobeCredentials> {
        store().lock().unwrap().clone()
    }
    pub(crate) fn write(list: &[KobeCredentials]) -> bool {
        *store().lock().unwrap() = list.to_vec();
        true
    }
}

/// 记住的账号列表(按存入顺序)。
pub fn list() -> Vec<KobeCredentials> {
    imp::read()
}

/// 新增 / 更新一个账号(按 email 去重覆盖)。`creds.auto_login` 为真时,把其余账号的
/// 自动登录标记一律清掉(自动登录唯一)。keyring 不可用时返回 `false`(不落明文)。
pub fn upsert(creds: &KobeCredentials) -> bool {
    let mut list = imp::read();
    list.retain(|c| c.email != creds.email);
    if creds.auto_login {
        for c in &mut list {
            c.auto_login = false;
        }
    }
    list.push(creds.clone());
    imp::write(&list)
}

/// 移除某账号(取消记住)。
pub fn remove(email: &str) -> bool {
    let mut list = imp::read();
    list.retain(|c| c.email != email);
    imp::write(&list)
}

/// 设置某账号是否自动登录。置真时其余账号的标记一律清掉(自动登录唯一)。
pub fn set_auto_login(email: &str, on: bool) -> bool {
    let mut list = imp::read();
    for c in &mut list {
        if c.email == email {
            c.auto_login = on;
        } else if on {
            c.auto_login = false;
        }
    }
    imp::write(&list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cred(email: &str, auto: bool) -> KobeCredentials {
        KobeCredentials { email: email.into(), password: "pw".into(), auto_login: auto }
    }

    #[test]
    fn upsert_dedups_by_email_and_keeps_single_auto() {
        remove("a@x"); // clean (process-shared test store)
        remove("b@x");
        assert!(upsert(&cred("a@x", true)));
        assert!(upsert(&cred("b@x", true))); // b auto → a's auto cleared
        let l = list();
        assert_eq!(l.iter().filter(|c| c.auto_login).count(), 1);
        assert!(l.iter().find(|c| c.email == "b@x").unwrap().auto_login);
        assert!(!l.iter().find(|c| c.email == "a@x").unwrap().auto_login);

        // re-upsert same email updates in place (no dup)
        assert!(upsert(&cred("a@x", false)));
        assert_eq!(list().iter().filter(|c| c.email == "a@x").count(), 1);

        set_auto_login("a@x", true);
        assert!(list().iter().find(|c| c.email == "a@x").unwrap().auto_login);
        assert!(!list().iter().find(|c| c.email == "b@x").unwrap().auto_login);

        remove("a@x");
        remove("b@x");
        assert!(list().is_empty());
    }
}
