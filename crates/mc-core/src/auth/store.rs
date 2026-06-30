//! 多账号持久化。
//!
//! 把账号列表(及当前选中项)存为一个 JSON 文件。所有账号类型(离线 / 微软 /
//! 外置)都归一到 [`StoredAccount`],并能导出统一的 [`AuthSession`] 给启动阶段
//! ——启动代码无需关心账号是哪种类型。
//!
//! # 安全
//!
//! 敏感 token(`access_token` / `refresh_token` / `client_token`)默认存进**系统原生
//! keyring**(macOS Keychain / Windows Credential Manager / Linux keyutils),`accounts.json`
//! 里只留非敏感元数据 + 一个「token 在 keyring」的标记。详见 [`super::secret`]。
//!
//! 这一切对调用方透明:加载后仍能直接从 [`StoredAccount`] 上读到 token。
//! - 旧版明文文件在加载时会**自动迁移**:把 token 写进 keyring,并改写文件清除明文。
//! - keyring 不可用(`keyring` feature 关闭、或运行期报错)时**优雅回退**到明文存储,
//!   绝不因此报错把用户锁在门外;此时文件仍含明文 token,故在 Unix 上把权限收紧到 `0600`。

use std::path::{Path, PathBuf};

use mc_types::{AccountKind, AccountSummary, AuthSession};
use serde::{Deserialize, Serialize};

use super::secret;
use super::yggdrasil::YggdrasilSession;
use super::MC_TOKEN_TTL_SECS;
use crate::error::{CoreError, IoResultExt, Result};

/// 单个持久化账号。涵盖三种账号类型的全部字段(按需填充)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredAccount {
    pub kind: AccountKind,
    pub username: String,
    pub uuid: String,
    /// 游戏可用的 access token(离线为占位 "0")。
    ///
    /// 安全提示:落盘时此字段(及下面的 refresh/client token)默认移入系统 keyring,
    /// 文件里只留标记;见模块文档与 [`super::secret`]。
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
    /// access_token 的预计过期时间(Unix 秒)。微软账号据此判断是否需要用 refresh_token
    /// 续期;`None`(老数据 / 离线)视为「未知」,需要时直接尝试续期。
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// 外置登录(Yggdrasil)的 clientToken,authenticate/refresh/validate 之间必须一致;
    /// 非外置账号为 `None`。
    #[serde(default)]
    pub client_token: Option<String>,
    /// 外置登录的 authserver 根地址(如 `https://littleskin.cn/api/yggdrasil`),
    /// 启动时据此注入 authlib-injector 的 `-javaagent`;非外置账号为 `None`。
    #[serde(default)]
    pub yggdrasil_base: Option<String>,
}

impl StoredAccount {
    /// 由微软登录结果构造一个可落库的微软账号。初次设备码登录意味着账号必然拥有正版,
    /// 故 `owns_game` 恒为 `true`——其余字段布局与 TTL 全交给
    /// [`from_microsoft_refreshed`](Self::from_microsoft_refreshed) 这个唯一 owner。
    pub fn from_microsoft(session: &AuthSession, refresh_token: String) -> Self {
        Self::from_microsoft_refreshed(session, refresh_token, true)
    }

    /// 「AuthSession + refresh_token + owns_game → 微软 StoredAccount」的唯一字段布局 & TTL
    /// owner:`expires_at` 统一用 [`MC_TOKEN_TTL_SECS`] 在构造时计算,desktop / CLI 初次登录
    /// ([`from_microsoft`](Self::from_microsoft))与续期路径([`super::refresh_selected_microsoft`])
    /// 共用同一份,不再各处手抄字段、更不会再出现把 TTL 写成 `86_400` 字面量的漂移。
    ///
    /// 与初次登录的唯一区别是 `owns_game` 由调用方显式给定(续期沿用旧账号,而非强制 `true`);
    /// refresh_token 的「端点未返回新值则沿用旧值」回退也由调用方先解析好再传入,故这里只收
    /// 已定稿的 token。
    pub(crate) fn from_microsoft_refreshed(
        session: &AuthSession,
        refresh_token: String,
        owns_game: bool,
    ) -> Self {
        Self {
            kind: AccountKind::Microsoft,
            username: session.username.clone(),
            uuid: session.uuid.clone(),
            access_token: session.access_token.clone(),
            refresh_token: Some(refresh_token),
            xuid: session.xuid.clone(),
            user_type: session.user_type.clone(),
            owns_game,
            expires_at: Some(now_unix() + MC_TOKEN_TTL_SECS),
            client_token: None,
            yggdrasil_base: None,
        }
    }

    /// 由离线 session 构造离线账号(无 token 续期、不预判过期、不拥有正版)。
    pub fn from_offline(session: &AuthSession) -> Self {
        Self {
            kind: AccountKind::Offline,
            username: session.username.clone(),
            uuid: session.uuid.clone(),
            access_token: session.access_token.clone(),
            refresh_token: None,
            xuid: session.xuid.clone(),
            user_type: session.user_type.clone(),
            owns_game: false,
            expires_at: None,
            client_token: None,
            yggdrasil_base: None,
        }
    }

    /// 由外置(Yggdrasil)登录结果 + authserver `base` 构造外置账号:持久化 `client_token`
    /// (续期所需)与 `base`(启动时注入 authlib-injector 所需)。token 由皮肤站签发,
    /// 这里不预判过期。
    pub fn from_yggdrasil(session: &YggdrasilSession, base: String) -> Self {
        Self {
            kind: AccountKind::Yggdrasil,
            username: session.username.clone(),
            uuid: session.uuid.clone(),
            access_token: session.access_token.clone(),
            refresh_token: None,
            xuid: String::new(),
            user_type: "msa".to_string(),
            owns_game: true,
            expires_at: None,
            client_token: Some(session.client_token.clone()),
            yggdrasil_base: Some(base),
        }
    }

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

    /// access_token 是否（接近）过期:留 `margin_secs` 提前量,避免临界点启动时刚好失效。
    /// `expires_at` 为 `None`（未知)时一律视为已过期,促使尽早续期。
    pub fn is_expired(&self, now_unix: i64, margin_secs: i64) -> bool {
        match self.expires_at {
            Some(exp) => now_unix >= exp - margin_secs,
            None => true,
        }
    }
}

/// 把文件权限收紧为「仅属主可读写」(Unix `0o600`)。非 Unix 平台为 no-op。
/// 尽力而为:取不到/设不了权限都静默忽略(不阻断保存)。
fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// 当前 Unix 时间(秒)。系统时钟早于纪元的极端情况回退 0。
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 磁盘上的存储格式(账号列表 + 选中项)。单独成型,便于将来演进 schema。
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    accounts: Vec<OnDiskAccount>,
    /// 当前选中账号的 uuid。
    #[serde(default)]
    selected: Option<String>,
}

/// 账号的磁盘表示:[`StoredAccount`] 的全部字段(`flatten` 内联),外加一个标记位——
/// 标记位为真时,敏感 token 存在系统 keyring,内联的 token 字段已被清空。
///
/// 用独立的 on-disk 结构(而非给 `StoredAccount` 加字段)是为了让公开的
/// `StoredAccount` 形状保持不变,`mc-cli` / `launch` 等调用方无需改动。
#[derive(Debug, Serialize, Deserialize)]
struct OnDiskAccount {
    #[serde(flatten)]
    account: StoredAccount,
    /// 敏感 token 是否存放在系统 keyring(为真时 JSON 内对应字段已清空)。
    #[serde(default, skip_serializing_if = "is_false")]
    secrets_in_keyring: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if 要求 &bool
fn is_false(b: &bool) -> bool {
    !*b
}

/// 该账号是否带「真正的」敏感 token,值得放进 keyring。离线账号的占位 `"0"`(或空)
/// 不算秘密,继续留在明文文件里即可,避免给钥匙串塞无意义条目。
fn has_real_secrets(a: &StoredAccount) -> bool {
    a.refresh_token.is_some()
        || a.client_token.is_some()
        || (!a.access_token.is_empty() && a.access_token != "0")
}

/// 把内存账号转成磁盘表示:有真实 token 且 keyring 写入成功 → 清空内联 token、置标记;
/// 否则(无敏感 token,或 keyring 不可用/失败)→ 原样明文存储。
fn to_on_disk(a: &StoredAccount) -> OnDiskAccount {
    if has_real_secrets(a) {
        let secrets = secret::Secrets {
            access_token: a.access_token.clone(),
            refresh_token: a.refresh_token.clone(),
            client_token: a.client_token.clone(),
        };
        if secret::write(&a.uuid, &secrets) {
            let mut redacted = a.clone();
            redacted.access_token = String::new();
            redacted.refresh_token = None;
            redacted.client_token = None;
            return OnDiskAccount {
                account: redacted,
                secrets_in_keyring: true,
            };
        }
        // keyring 不可用/写入失败:回退明文,绝不丢 token。
        tracing::warn!(uuid = %a.uuid, "keyring 写入失败,账号 token 回退明文存储");
    }
    OnDiskAccount {
        account: a.clone(),
        secrets_in_keyring: false,
    }
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

        // 把磁盘表示还原成内存账号:keyring 里的 token 取回填上;旧版明文(标记为假但带真实
        // token)记下来,稍后整体迁移进 keyring 并改写文件。
        let mut needs_migration = false;
        let mut accounts = Vec::with_capacity(file.accounts.len());
        for od in file.accounts {
            let mut acct = od.account;
            if od.secrets_in_keyring {
                if let Some(s) = secret::read(&acct.uuid) {
                    acct.access_token = s.access_token;
                    acct.refresh_token = s.refresh_token;
                    acct.client_token = s.client_token;
                }
                // 取不到(条目被删等):保留已清空的字段,best-effort 不报错。
            } else if secret::available() && has_real_secrets(&acct) {
                needs_migration = true;
            }
            accounts.push(acct);
        }

        let mut store = Self {
            path,
            accounts,
            selected: file.selected,
        };
        // 修正选中项:若选中的 uuid 已不存在则清空,避免悬空引用。
        store.normalize_selected();

        // 旧版明文文件:把 token 迁入 keyring 并改写文件清除明文(save 自带回退,失败则保持明文)。
        if needs_migration {
            match store.save() {
                Ok(()) => tracing::info!("已把账号明文 token 迁移进系统 keyring"),
                Err(e) => tracing::warn!(error = %e, "迁移账号 token 进 keyring 失败,暂留明文"),
            }
        }
        Ok(store)
    }

    /// 把当前账号列表与选中项写回磁盘(美化 JSON)。父目录由 write_atomic 自动创建。
    pub fn save(&self) -> Result<()> {
        // 敏感 token 写入 keyring(成功则文件里清空),只把元数据 + 标记落盘。
        let file = StoreFile {
            accounts: self.accounts.iter().map(to_on_disk).collect(),
            selected: self.selected.clone(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(|e| CoreError::Parse {
            what: "serialize account store".to_string(),
            source: e,
        })?;
        crate::fs::write_atomic(&self.path, text.as_bytes())?;
        // token 优先存 keyring;但 keyring 不可用时会回退明文,且文件始终可能含离线占位等
        // 元数据,故在 Unix 上仍把权限收紧到「仅属主可读写」(0600)。
        restrict_to_owner(&self.path);
        Ok(())
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
        if removed {
            // 账号没了就清掉它在 keyring 里的 token 条目(best-effort)。
            secret::delete(uuid);
            if self.selected.as_deref() == Some(uuid) {
                self.selected = self.accounts.first().map(|a| a.uuid.clone());
            }
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

    /// 加入(或按 uuid 原地更新)一个账号、选中它、并落盘——这三步是登录 / 续期场景里
    /// **一个不可分的动作**。此前每个调用方都得自己按 add → select → save 的顺序手写,漏一
    /// 步或写错序就会出现「加了不选中」或「选了不保存」。`select` 必成功(刚 add 过该 uuid),
    /// 这里把它的 `Err` 一并上抛而非各调用方时灵时不灵地忽略。
    pub fn add_and_select(&mut self, account: StoredAccount) -> Result<()> {
        let uuid = account.uuid.clone();
        self.add(account);
        self.select(&uuid)?;
        self.save()
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

    /// `cfg(test)` 下 [`secret`] 后端自动换成进程内内存 store(见该模块),测试绝不触碰真实
    /// 钥匙串。此处保留一个显式的 no-op,标注「这个测试会走 keyring 路径」。
    fn init_mock_keyring() {}

    #[test]
    fn is_expired_respects_margin_and_unknown() {
        let mut a = offline("x", "u");
        a.expires_at = Some(1_000);
        // 1000 到期、margin 60:940 起即视为(接近)过期。
        assert!(!a.is_expired(939, 60));
        assert!(a.is_expired(940, 60));
        assert!(a.is_expired(1_000, 60));
        // expires_at 未知 → 一律视为过期(促使续期)。
        a.expires_at = None;
        assert!(a.is_expired(0, 0));
    }

    fn ms_session() -> AuthSession {
        AuthSession {
            username: "msuser".to_string(),
            uuid: "uuid-ms".to_string(),
            access_token: "acc".to_string(),
            user_type: "msa".to_string(),
            xuid: "x1".to_string(),
        }
    }

    /// 初次登录与续期两条路径用同样的输入(owns_game 对齐)应得到同样的字段布局 + 同样的 TTL。
    /// 这是去重的核心保证:`from_microsoft` 只是 `from_microsoft_refreshed(.., true)` 的薄包装,
    /// 字段布局 / TTL 只此一份。
    #[test]
    fn microsoft_initial_and_refresh_paths_match() {
        let session = ms_session();
        let before = now_unix();
        let initial = StoredAccount::from_microsoft(&session, "refresh".to_string());
        let refreshed =
            StoredAccount::from_microsoft_refreshed(&session, "refresh".to_string(), true);
        let after = now_unix();

        // TTL:两条路径都把 expires_at 设为各自构造时刻 + MC_TOKEN_TTL_SECS。
        for acc in [&initial, &refreshed] {
            let exp = acc.expires_at.expect("微软账号应有 expires_at");
            assert!(exp >= before + MC_TOKEN_TTL_SECS && exp <= after + MC_TOKEN_TTL_SECS);
        }
        // 除 expires_at(各自取构造时刻)外其余字段逐一相同 —— 字段布局只此一份。
        let norm = |mut a: StoredAccount| {
            a.expires_at = None;
            a
        };
        assert_eq!(norm(initial.clone()), norm(refreshed.clone()));

        // 字段布局 spot-check:确实是拥有正版、带 refresh_token、msa 的微软账号,且无外置字段。
        assert_eq!(initial.kind, AccountKind::Microsoft);
        assert!(initial.owns_game);
        assert_eq!(initial.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(initial.user_type, "msa");
        assert_eq!(initial.xuid, "x1");
        assert!(initial.client_token.is_none());
        assert!(initial.yggdrasil_base.is_none());
    }

    /// 续期路径相对初次登录的真实差异:`owns_game` 沿用旧账号(可能为 false),而非强制 `true`。
    #[test]
    fn microsoft_refreshed_carries_owns_game() {
        let acc = StoredAccount::from_microsoft_refreshed(&ms_session(), "r".to_string(), false);
        assert!(!acc.owns_game);
    }

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
            expires_at: None,
            client_token: None,
            yggdrasil_base: None,
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
    fn add_and_select_switches_selection_to_each_new_account() {
        // 落盘到唯一临时路径,避免与并发测试争用固定文件。
        let path = std::env::temp_dir()
            .join(format!("mc-store-addsel-{}.json", std::process::id()));
        let mut s = AccountStore { path, accounts: Vec::new(), selected: None };
        s.add_and_select(offline("alice", "uuid-a")).unwrap();
        s.add_and_select(offline("bob", "uuid-b")).unwrap();
        // `add` 单独只在列表原本为空时自动选中第一个;add_and_select 必须把选中切到
        // **每个**新加入的账号 —— 这正是所有登录调用方依赖、却各自手写易漏的那一步。
        assert_eq!(s.selected.as_deref(), Some("uuid-b"));
    }

    #[test]
    fn remove_selected_falls_back_to_first() {
        init_mock_keyring();
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
        init_mock_keyring();
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
        init_mock_keyring();
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
            expires_at: None,
            client_token: None,
            yggdrasil_base: None,
        });
        s.select("uuid-ms").unwrap();
        s.save().unwrap();

        // 开启 keyring(mock)时:磁盘上不应再出现明文 token,而是带 keyring 标记。
        #[cfg(feature = "keyring")]
        {
            let on_disk = std::fs::read_to_string(&p).unwrap();
            assert!(
                !on_disk.contains("mctoken") && !on_disk.contains("\"refresh\""),
                "敏感 token 不应明文落盘:{on_disk}"
            );
            assert!(on_disk.contains("secrets_in_keyring"));
        }

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

        // 含明文 token 的账号库应被收紧为仅属主可读写(0600)。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "accounts.json 应为 0600,实际 {:o}", mode & 0o777);
        }

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

    /// 离线账号的占位 token("0")不算秘密:始终留在明文文件里,不进 keyring,
    /// 加载/保存照常往返。feature 开关都成立。
    #[test]
    fn offline_token_stays_plaintext() {
        init_mock_keyring();
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-offline-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&p);

        let mut s = AccountStore::load(&p).unwrap();
        s.add(offline("alice", "uuid-off"));
        s.save().unwrap();

        let on_disk = std::fs::read_to_string(&p).unwrap();
        assert!(on_disk.contains("\"access_token\": \"0\""));
        assert!(!on_disk.contains("secrets_in_keyring"));

        let loaded = AccountStore::load(&p).unwrap();
        assert_eq!(loaded.selected_session().unwrap().access_token, "0");
        let _ = std::fs::remove_file(&p);
    }

    /// 旧版明文文件(token 在 JSON 里、无 keyring 标记)在加载时应自动迁移:token 进 keyring,
    /// 文件被改写清除明文;之后重新加载仍能从 keyring 取回 token。仅在 keyring feature 开启时成立。
    #[cfg(feature = "keyring")]
    #[test]
    fn legacy_plaintext_migrates_into_keyring() {
        init_mock_keyring();
        let p = std::env::temp_dir().join(format!(
            "mc-core-auth-store-migrate-{}.json",
            std::process::id()
        ));
        let json = r#"{
            "accounts": [
                {"kind":"microsoft","username":"legacy","uuid":"uuid-legacy",
                 "access_token":"plain-access","refresh_token":"plain-refresh",
                 "xuid":"x1","user_type":"msa","owns_game":true}
            ],
            "selected": "uuid-legacy"
        }"#;
        std::fs::write(&p, json).unwrap();

        // 加载触发迁移:内存里 token 仍正确。
        let loaded = AccountStore::load(&p).unwrap();
        let acc = loaded.selected_account().unwrap();
        assert_eq!(acc.access_token, "plain-access");
        assert_eq!(acc.refresh_token.as_deref(), Some("plain-refresh"));

        // 文件已被改写:不再含明文 token,改为 keyring 标记。
        let on_disk = std::fs::read_to_string(&p).unwrap();
        assert!(
            !on_disk.contains("plain-access") && !on_disk.contains("plain-refresh"),
            "迁移后明文 token 不应残留:{on_disk}"
        );
        assert!(on_disk.contains("secrets_in_keyring"));

        // 全新加载:token 从 keyring 取回。
        let reloaded = AccountStore::load(&p).unwrap();
        let acc = reloaded.selected_account().unwrap();
        assert_eq!(acc.access_token, "plain-access");
        assert_eq!(acc.refresh_token.as_deref(), Some("plain-refresh"));

        let _ = std::fs::remove_file(&p);
    }
}
