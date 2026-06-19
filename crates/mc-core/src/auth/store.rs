//! 多账号持久化。
//!
//! 把账号列表(及当前选中项)存为一个 JSON 文件。所有账号类型(离线 / 微软 /
//! 外置)都归一到 [`StoredAccount`],并能导出统一的 [`AuthSession`] 给启动阶段
//! ——启动代码无需关心账号是哪种类型。
//!
//! # 安全(TODO)
//!
//! 当前实现把 `access_token` / `refresh_token` **明文**写入磁盘。生产环境必须
//! 改为平台原生的安全存储:
//! - macOS:Keychain
//! - Windows:DPAPI / Credential Manager
//! - Linux:Secret Service (libsecret)
//!
//! 应通过系统 keyring 保存敏感 token,文件里只留非敏感元数据(uuid / 用户名 /
//! 所有权)。这需要引入 `keyring` 之类的依赖,本 crate 暂未引入,故先明文存储。
//! **切勿在生产中以明文形式分发此文件。**

use std::path::{Path, PathBuf};

use mc_types::{AccountKind, AccountSummary, AuthSession};
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoResultExt, Result};
use crate::paths::ensure_dir;

/// 单个持久化账号。涵盖三种账号类型的全部字段(按需填充)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredAccount {
    pub kind: AccountKind,
    pub username: String,
    pub uuid: String,
    /// 游戏可用的 access token(离线为占位 "0")。
    ///
    /// 安全提示:见模块文档,生产应改为系统 keyring 存储。
    pub access_token: String,
    /// 微软账号的刷新令牌,用于免浏览器续期;其他类型为 `None`。
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Xbox 用户 id(`${auth_xuid}`),非微软账号为空串。
    #[serde(default)]
    pub xuid: String,
    /// 传给游戏的 `${user_type}`:`msa` 或 `legacy`。
    pub user_type: String,
    /// 是否拥有正版游戏(仅微软账号有意义)。
    #[serde(default)]
    pub owns_game: bool,
}

impl StoredAccount {
    /// 从本账号构造启动用的 [`AuthSession`]。
    pub fn to_session(&self) -> AuthSession {
        AuthSession {
            username: self.username.clone(),
            uuid: self.uuid.clone(),
            access_token: self.access_token.clone(),
            user_type: self.user_type.clone(),
            xuid: self.xuid.clone(),
        }
    }
}

/// 磁盘上的存储格式(账号列表 + 选中项)。单独成型,便于将来演进 schema。
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    accounts: Vec<StoredAccount>,
    /// 当前选中账号的 uuid。
    #[serde(default)]
    selected: Option<String>,
}

/// 账号存储。内存中保存账号列表与选中项,[`save`](Self::save) 落盘。
#[derive(Debug, Clone)]
pub struct AccountStore {
    path: PathBuf,
    accounts: Vec<StoredAccount>,
    selected: Option<String>,
}

impl AccountStore {
    /// 从 `path` 加载账号存储。文件不存在时返回空存储(首次启动场景),
    /// 不视为错误。
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                accounts: Vec::new(),
                selected: None,
            });
        }

        let text = std::fs::read_to_string(&path).with_path(&path)?;
        // 空文件也按空存储处理,避免解析报错。
        if text.trim().is_empty() {
            return Ok(Self {
                path,
                accounts: Vec::new(),
                selected: None,
            });
        }

        let file: StoreFile = serde_json::from_str(&text).map_err(|e| CoreError::Parse {
            what: format!("account store {}", path.display()),
            source: e,
        })?;

        let mut store = Self {
            path,
            accounts: file.accounts,
            selected: file.selected,
        };
        // 修正选中项:若选中的 uuid 已不存在则清空,避免悬空引用。
        store.normalize_selected();
        Ok(store)
    }

    /// 把当前账号列表与选中项写回磁盘(美化 JSON)。必要时创建父目录。
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            ensure_dir(parent)?;
        }
        let file = StoreFile {
            accounts: self.accounts.clone(),
            selected: self.selected.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| CoreError::Parse {
            what: "serialize account store".to_string(),
            source: e,
        })?;
        crate::fs::write_atomic(&self.path, text.as_bytes())
    }

    /// 加入(或更新)一个账号。若已存在相同 uuid 则原地替换(刷新 token 场景);
    /// 否则追加。第一个加入的账号自动成为选中项。
    pub fn add(&mut self, account: StoredAccount) {
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.uuid == account.uuid) {
            *existing = account;
        } else {
            // 列表原本为空时,新账号默认选中。
            if self.accounts.is_empty() {
                self.selected = Some(account.uuid.clone());
            }
            self.accounts.push(account);
        }
    }

    /// 按 uuid 移除账号。若移除的是当前选中项,选中项回退到列表第一个(若有)。
    /// 返回是否确有账号被移除。
    pub fn remove(&mut self, uuid: &str) -> bool {
        let before = self.accounts.len();
        self.accounts.retain(|a| a.uuid != uuid);
        let removed = self.accounts.len() != before;
        if removed && self.selected.as_deref() == Some(uuid) {
            self.selected = self.accounts.first().map(|a| a.uuid.clone());
        }
        removed
    }

    /// 选中指定 uuid 的账号。uuid 不存在时返回 [`CoreError::Auth`]。
    pub fn select(&mut self, uuid: &str) -> Result<()> {
        if self.accounts.iter().any(|a| a.uuid == uuid) {
            self.selected = Some(uuid.to_string());
            Ok(())
        } else {
            Err(CoreError::Auth(format!("账号 {uuid} 不存在,无法选中")))
        }
    }

    /// 以 [`AccountSummary`] 形式列出全部账号(供账号切换器展示)。
    pub fn list(&self) -> Vec<AccountSummary> {
        self.accounts
            .iter()
            .map(|a| AccountSummary {
                kind: a.kind,
                username: a.username.clone(),
                uuid: a.uuid.clone(),
                selected: self.selected.as_deref() == Some(a.uuid.as_str()),
                owns_game: a.owns_game,
            })
            .collect()
    }

    /// 取当前选中账号的 [`AuthSession`];无选中项时返回 `None`。
    pub fn selected_session(&self) -> Option<AuthSession> {
        self.selected_account().map(StoredAccount::to_session)
    }

    /// 取当前选中的 [`StoredAccount`] 引用(需要 refresh_token 续期时用)。
    pub fn selected_account(&self) -> Option<&StoredAccount> {
        let uuid = self.selected.as_deref()?;
        self.accounts.iter().find(|a| a.uuid == uuid)
    }

    /// 全部账号的只读切片。
    pub fn accounts(&self) -> &[StoredAccount] {
        &self.accounts
    }

    /// 存储文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 若选中项指向已不存在的 uuid,则回退到第一个账号(或 `None`)。
    fn normalize_selected(&mut self) {
        let valid = self
            .selected
            .as_deref()
            .map(|sel| self.accounts.iter().any(|a| a.uuid == sel))
            .unwrap_or(false);
        if !valid {
            self.selected = self.accounts.first().map(|a| a.uuid.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline(name: &str, uuid: &str) -> StoredAccount {
        StoredAccount {
            kind: AccountKind::Offline,
            username: name.to_string(),
            uuid: uuid.to_string(),
            access_token: "0".to_string(),
            refresh_token: None,
            xuid: String::new(),
            user_type: "legacy".to_string(),
            owns_game: false,
        }
    }

    fn empty_store() -> AccountStore {
        AccountStore {
            path: PathBuf::from("/tmp/does-not-matter.json"),
            accounts: Vec::new(),
            selected: None,
        }
    }

    #[test]
    fn first_added_becomes_selected() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        assert_eq!(s.selected.as_deref(), Some("uuid-a"));
        let sess = s.selected_session().unwrap();
        assert_eq!(sess.username, "alice");
        assert_eq!(sess.user_type, "legacy");
    }

    #[test]
    fn add_existing_uuid_replaces_in_place() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        s.add(offline("bob", "uuid-b"));
        // 用同一 uuid 再 add,应替换而非新增。
        let mut updated = offline("alice2", "uuid-a");
        updated.access_token = "newtok".to_string();
        s.add(updated);
        assert_eq!(s.accounts.len(), 2);
        let a = s.accounts.iter().find(|a| a.uuid == "uuid-a").unwrap();
        assert_eq!(a.username, "alice2");
        assert_eq!(a.access_token, "newtok");
    }

    #[test]
    fn select_unknown_uuid_errors() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        assert!(s.select("nope").is_err());
        assert!(s.select("uuid-a").is_ok());
    }

    #[test]
    fn remove_selected_falls_back_to_first() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        s.add(offline("bob", "uuid-b"));
        s.select("uuid-b").unwrap();
        assert!(s.remove("uuid-b"));
        // 选中项回退到剩余列表的第一个。
        assert_eq!(s.selected.as_deref(), Some("uuid-a"));
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        assert!(!s.remove("ghost"));
    }

    #[test]
    fn list_marks_selected() {
        let mut s = empty_store();
        s.add(offline("alice", "uuid-a"));
        s.add(offline("bob", "uuid-b"));
        s.select("uuid-b").unwrap();
        let list = s.list();
        let a = list.iter().find(|x| x.uuid == "uuid-a").unwrap();
        let b = list.iter().find(|x| x.uuid == "uuid-b").unwrap();
        assert!(!a.selected);
        assert!(b.selected);
    }

    #[test]
    fn load_missing_file_is_empty() {
        let p = std::env::temp_dir().join("mc-core-auth-store-missing-xyz.json");
        let _ = std::fs::remove_file(&p);
        let s = AccountStore::load(&p).unwrap();
        assert!(s.accounts().is_empty());
        assert!(s.selected_session().is_none());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-roundtrip-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);

        let mut s = AccountStore::load(&p).unwrap();
        s.add(offline("alice", "uuid-a"));
        s.add(StoredAccount {
            kind: AccountKind::Microsoft,
            username: "msuser".to_string(),
            uuid: "uuid-ms".to_string(),
            access_token: "mctoken".to_string(),
            refresh_token: Some("refresh".to_string()),
            xuid: "xuid123".to_string(),
            user_type: "msa".to_string(),
            owns_game: true,
        });
        s.select("uuid-ms").unwrap();
        s.save().unwrap();

        let loaded = AccountStore::load(&p).unwrap();
        assert_eq!(loaded.accounts().len(), 2);
        assert_eq!(loaded.selected.as_deref(), Some("uuid-ms"));
        let sess = loaded.selected_session().unwrap();
        assert_eq!(sess.username, "msuser");
        assert_eq!(sess.user_type, "msa");
        assert_eq!(sess.xuid, "xuid123");
        let ms = loaded
            .accounts()
            .iter()
            .find(|a| a.uuid == "uuid-ms")
            .unwrap();
        assert_eq!(ms.refresh_token.as_deref(), Some("refresh"));
        assert!(ms.owns_game);

        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_drops_dangling_selected() {
        // 选中项指向不存在的 uuid 时,加载后应回退到第一个账号。
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-dangling-{}.json",
            std::process::id()
        ));
        let json = r#"{
            "accounts": [
                {"kind":"offline","username":"alice","uuid":"uuid-a",
                 "access_token":"0","user_type":"legacy"}
            ],
            "selected": "ghost"
        }"#;
        std::fs::write(&p, json).unwrap();
        let s = AccountStore::load(&p).unwrap();
        assert_eq!(s.selected.as_deref(), Some("uuid-a"));
        let _ = std::fs::remove_file(&p);
    }
}
