use super::*;

/// 拉取轻量后端(mc-server)的新闻/公告。后端未运行/不可达时返回错误,UI 降级到空/错误态。
#[tauri::command]
#[specta::specta]
pub async fn fetch_news() -> CmdResult<Vec<mc_core::server::NewsItem>> {
    let client = mc_core::server::ServerClient::new().map_err(err)?;
    client.news().await.map_err(err)
}

// --- kobeMC account (our backend: better-auth email/password) ----------------
// These reuse one shared ServerClient held in Tauri state (lib.rs `.manage`) so
// the better-auth session cookie persists across calls within an app session.

/// Build the shared mc-server client (managed in Tauri state).
pub fn kobe_client() -> mc_core::server::ServerClient {
    mc_core::server::ServerClient::new().expect("build mc-server client")
}

/// Register a kobeMC account (email/password); establishes the session.
#[tauri::command]
#[specta::specta]
pub async fn kobemc_signup(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    email: String,
    password: String,
    name: String,
) -> CmdResult<mc_core::server::AuthUser> {
    client.register(&email, &password, &name).await.map_err(err)
}

/// Log in to a kobeMC account; establishes the session cookie.
#[tauri::command]
#[specta::specta]
pub async fn kobemc_login(
    client: tauri::State<'_, mc_core::server::ServerClient>,
    email: String,
    password: String,
) -> CmdResult<mc_core::server::AuthUser> {
    client.login(&email, &password).await.map_err(err)
}

/// The current kobeMC session user, or `None` if not logged in.
#[tauri::command]
#[specta::specta]
pub async fn kobemc_session(
    client: tauri::State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Option<mc_core::server::AuthUser>> {
    Ok(client.me().await.ok())
}

/// Log out of the kobeMC account (clears the server session).
#[tauri::command]
#[specta::specta]
pub async fn kobemc_logout(
    client: tauri::State<'_, mc_core::server::ServerClient>,
) -> CmdResult<()> {
    client.logout().await.map_err(err)
}

/// Remember a kobeMC account (upsert by email) in the OS keyring. When the keyring
/// is unavailable nothing is written (no plaintext on disk). `auto_login = true`
/// makes this the single auto-login account (clears the flag on the others).
#[tauri::command]
#[specta::specta]
pub fn kobe_save_credentials(email: String, password: String, auto_login: bool) -> CmdResult<()> {
    mc_core::auth::kobe_creds::upsert(&mc_core::auth::kobe_creds::KobeCredentials {
        email,
        password,
        auto_login,
    });
    Ok(())
}

/// The list of remembered kobeMC accounts (may be empty).
#[tauri::command]
#[specta::specta]
pub fn kobe_list_credentials() -> CmdResult<Vec<mc_core::auth::kobe_creds::KobeCredentials>> {
    Ok(mc_core::auth::kobe_creds::list())
}

/// Forget a remembered kobeMC account by email.
#[tauri::command]
#[specta::specta]
pub fn kobe_remove_credentials(email: String) -> CmdResult<()> {
    mc_core::auth::kobe_creds::remove(&email);
    Ok(())
}

/// Toggle whether a remembered account auto-logs-in at startup (at most one).
#[tauri::command]
#[specta::specta]
pub fn kobe_set_auto_login(email: String, auto_login: bool) -> CmdResult<()> {
    mc_core::auth::kobe_creds::set_auto_login(&email, auto_login);
    Ok(())
}

