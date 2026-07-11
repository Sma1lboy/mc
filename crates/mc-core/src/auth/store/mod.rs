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
mod tests;
