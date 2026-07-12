use super::*;

#[tauri::command]
#[specta::specta]
pub fn list_accounts() -> CmdResult<Vec<AccountSummary>> {
    let store = AccountStore::load(accounts_path()).map_err(err)?;
    Ok(store.list())
}

// --- accounts: Microsoft login + management ------------------------------

/// Persist a freshly built account, make it the selected one, and return its
/// summary. Shared by Microsoft and offline login.
fn store_and_select(account: StoredAccount) -> CmdResult<AccountSummary> {
    let _ = paths::ensure_dir(&data_dir());
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    let uuid = account.uuid.clone();
    store.add_and_select(account).map_err(err)?;
    store
        .list()
        .into_iter()
        .find(|a| a.uuid == uuid)
        .ok_or_else(|| "登录成功但未能读回账号".to_string())
}

/// The device-code prompt shown to the user. `device_code` is the opaque handle
/// passed back to [`msa_login_poll`]; everything else is for display.
#[derive(Serialize, specta::Type)]
pub struct DeviceCodeDto {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,
    pub expires_in: u64,
}

/// Step ① of Microsoft login: start the device-code flow. The UI shows
/// `user_code` and opens `verification_uri`, then calls [`msa_login_poll`].
#[tauri::command]
#[specta::specta]
pub async fn msa_login_start() -> CmdResult<DeviceCodeDto> {
    let info = msa_client().device_code_start().await.map_err(err)?;
    Ok(DeviceCodeDto {
        user_code: info.user_code,
        verification_uri: info.verification_uri,
        device_code: info.device_code,
        interval: info.interval,
        expires_in: info.expires_in,
    })
}

/// Step ② of Microsoft login: block until the user finishes in the browser,
/// run the full Xbox → XSTS → Minecraft → profile chain, then persist and
/// select the resulting account.
#[tauri::command]
#[specta::specta]
pub async fn msa_login_poll(device_code: String, interval: u64) -> CmdResult<AccountSummary> {
    let client = msa_client();
    let token = client.poll_token(&device_code, interval).await.map_err(err)?;
    let session = client.authenticate(&token.access_token).await.map_err(err)?;
    store_and_select(StoredAccount::from_microsoft(&session, token.refresh_token))
}

/// Add (or update) an offline account from a username and select it.
#[tauri::command]
#[specta::specta]
pub fn add_offline_account(name: String) -> CmdResult<AccountSummary> {
    let name = name.trim();
    if name.is_empty() {
        return Err("用户名不能为空".to_string());
    }
    let session = auth::offline_session(name);
    store_and_select(StoredAccount::from_offline(&session))
}

/// 外置登录(Yggdrasil / authlib-injector):用 base + 用户名 + 密码登录第三方皮肤站,
/// 落库为 Yggdrasil 账号并选中。启动时会自动注入 authlib-injector。
#[tauri::command]
#[specta::specta]
pub async fn yggdrasil_login(
    base: String,
    username: String,
    password: String,
) -> CmdResult<AccountSummary> {
    use mc_core::auth::YggdrasilClient;
    let base = base.trim();
    if base.is_empty() || username.trim().is_empty() {
        return Err("皮肤站地址和用户名不能为空".to_string());
    }
    let client = YggdrasilClient::new(base).with_http(make_downloader()?.client().clone());
    let session = client.authenticate(username.trim(), &password).await.map_err(err)?;
    store_and_select(StoredAccount::from_yggdrasil(&session, client.base().to_string()))
}

/// Switch the active account.
#[tauri::command]
#[specta::specta]
pub fn select_account(uuid: String) -> CmdResult<()> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    store.select(&uuid).map_err(err)?;
    store.save().map_err(err)
}

/// Remove an account by uuid.
#[tauri::command]
#[specta::specta]
pub fn remove_account(uuid: String) -> CmdResult<()> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    store.remove(&uuid);
    store.save().map_err(err)
}

/// 显式刷新当前选中的微软账号的登录(免浏览器,用 refresh_token)。返回是否执行了续期。
/// 失败(refresh_token 失效)时返回错误,UI 据此提示重新登录。
#[tauri::command]
#[specta::specta]
pub async fn refresh_account() -> CmdResult<bool> {
    let mut store = AccountStore::load(accounts_path()).map_err(err)?;
    // 显式刷新:用极大 margin 强制对选中的微软账号尝试续期,不看剩余有效期。
    auth::refresh_selected_microsoft(&mut store, &msa_client(), i64::MAX / 2)
        .await
        .map_err(err)
}

// --- skin / cape management (Microsoft accounts only) --------------------

/// 解析指定 uuid 账号的 Minecraft access token。仅微软账号有皮肤 API;
/// 离线 / 外置账号返回清晰错误(占位 token 用不了 profile 端点)。
fn mc_access_token(uuid: &str) -> CmdResult<String> {
    let store = AccountStore::load(accounts_path()).map_err(err)?;
    let acc = store
        .accounts()
        .iter()
        .find(|a| a.uuid == uuid)
        .ok_or_else(|| format!("账号 {uuid} 不存在"))?;
    if acc.kind != AccountKind::Microsoft {
        return Err("只有微软正版账号才能管理皮肤与披风".to_string());
    }
    if acc.access_token.is_empty() || acc.access_token == "0" {
        return Err("该账号缺少有效的登录令牌,请重新登录微软账号".to_string());
    }
    Ok(acc.access_token.clone())
}

/// 读取某微软账号的皮肤 / 披风资料。
#[tauri::command]
#[specta::specta]
pub async fn skin_profile(account_uuid: String) -> CmdResult<mc_core::skin::ProfileSkins> {
    let token = mc_access_token(&account_uuid)?;
    mc_core::skin::fetch_profile(&token).await.map_err(err)
}

/// 上传本地 PNG 作为新皮肤。`variant` 为 `classic` / `slim`。返回更新后的资料。
#[tauri::command]
#[specta::specta]
pub async fn skin_upload(
    account_uuid: String,
    path: String,
    variant: String,
) -> CmdResult<mc_core::skin::ProfileSkins> {
    let token = mc_access_token(&account_uuid)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("读取皮肤文件失败:{e}"))?;
    mc_core::skin::upload_skin(&token, &bytes, &variant).await.map_err(err)
}

/// 设置当前披风(`Some`)或隐藏披风(`None`)。返回更新后的资料。
#[tauri::command]
#[specta::specta]
pub async fn skin_set_cape(
    account_uuid: String,
    cape_id: Option<String>,
) -> CmdResult<mc_core::skin::ProfileSkins> {
    let token = mc_access_token(&account_uuid)?;
    mc_core::skin::set_cape(&token, cape_id.as_deref()).await.map_err(err)
}

