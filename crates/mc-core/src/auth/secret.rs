//! 账号敏感 token 的安全存储抽象。
//!
//! 把 `access_token` / `refresh_token` / `client_token` 从明文 `accounts.json` 挪进系统
//! 原生 keyring(macOS Keychain / Windows Credential Manager / Linux keyutils),磁盘上只
//! 保留非敏感元数据。每个账号的全部 token 合并成一条 JSON,存在 keyring 的单个条目下,
//! 条目键为 `(SERVICE, account_uuid)`。
//!
//! 设计要点:
//! - **不会锁死用户**:keyring 不可用(feature 关闭、或运行期报错)时,所有操作优雅回退到
//!   明文——保存就继续写明文,读取就用文件里的明文,绝不 `Err`。
//! - **feature 门控**:`keyring` feature(默认开启)关闭后整个依赖被剔除,`available()` 恒为
//!   `false`,行为等价于纯明文存储,方便 headless / 裁剪构建。
//! - **测试用内存 store**:`cfg(test)` 下后端换成进程内共享 `HashMap`(keyring crate 的
//!   `mock` 每个 `Entry` 各自独立、跨 `Entry::new` 不共享状态,无法做 load/save 往返,故用与之
//!   等价的内存实现),保证测试**绝不触碰真实钥匙串**,同时能验证存→取往返与迁移逻辑。

use serde::{Deserialize, Serialize};

/// 一个账号的全部敏感 token,作为整体存进 keyring 的单条目。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Secrets {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub client_token: Option<String>,
}

#[cfg(all(feature = "keyring", not(test)))]
mod imp {
    use super::Secrets;

    /// keyring 里的服务名(条目的另一半键是账号 uuid)。
    const SERVICE: &str = "mc-launcher";

    /// keyring 后端是否编译进来了(feature 开 → true)。仅表示「有后端可试」,
    /// 运行期具体读写仍可能失败并回退。
    pub(crate) fn available() -> bool {
        true
    }

    /// 读取某账号的 token;条目不存在或任何错误都返回 `None`(交由调用方回退明文)。
    pub(crate) fn read(uuid: &str) -> Option<Secrets> {
        let entry = keyring::Entry::new(SERVICE, uuid).ok()?;
        match entry.get_password() {
            Ok(blob) => serde_json::from_str(&blob).ok(),
            Err(_) => None,
        }
    }

    /// 写入某账号的 token,成功返回 `true`;失败(keyring 不可用等)返回 `false`,
    /// 调用方据此回退到明文存储。
    pub(crate) fn write(uuid: &str, secrets: &Secrets) -> bool {
        let entry = match keyring::Entry::new(SERVICE, uuid) {
            Ok(e) => e,
            Err(_) => return false,
        };
        let blob = match serde_json::to_string(secrets) {
            Ok(b) => b,
            Err(_) => return false,
        };
        entry.set_password(&blob).is_ok()
    }

    /// 尽力删除某账号的 keyring 条目(账号被移除时清理);不存在/失败都静默忽略。
    pub(crate) fn delete(uuid: &str) {
        if let Ok(entry) = keyring::Entry::new(SERVICE, uuid) {
            let _ = entry.delete_credential();
        }
    }
}

#[cfg(not(feature = "keyring"))]
mod imp {
    use super::Secrets;

    /// feature 关闭:无 keyring 后端,全部回退到明文。
    pub(crate) fn available() -> bool {
        false
    }

    pub(crate) fn read(_uuid: &str) -> Option<Secrets> {
        None
    }

    pub(crate) fn write(_uuid: &str, _secrets: &Secrets) -> bool {
        false
    }

    pub(crate) fn delete(_uuid: &str) {}
}

// 测试后端:进程内共享内存 store,跨 `Entry`(此处即跨 read/write 调用)持久,绝不触碰
// 真实钥匙串。仅在 `keyring` feature 开启的测试构建里启用(等价于 keyring 的 mock)。
#[cfg(all(feature = "keyring", test))]
mod imp {
    use super::Secrets;
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    fn store() -> &'static Mutex<HashMap<String, Secrets>> {
        static S: OnceLock<Mutex<HashMap<String, Secrets>>> = OnceLock::new();
        S.get_or_init(|| Mutex::new(HashMap::new()))
    }

    pub(crate) fn available() -> bool {
        true
    }

    pub(crate) fn read(uuid: &str) -> Option<Secrets> {
        store().lock().unwrap().get(uuid).cloned()
    }

    pub(crate) fn write(uuid: &str, secrets: &Secrets) -> bool {
        store()
            .lock()
            .unwrap()
            .insert(uuid.to_string(), secrets.clone());
        true
    }

    pub(crate) fn delete(uuid: &str) {
        store().lock().unwrap().remove(uuid);
    }
}

pub(crate) use imp::{available, delete, read, write};
