use super::*;

// --- friends (username search + request/accept over the held kobeMC client) ---

use mc_core::friend::UserBrief;

/// Set the current user's username (required before friend search works).
#[tauri::command]
#[specta::specta]
pub async fn friend_set_username(
    client: State<'_, mc_core::server::ServerClient>,
    username: String,
) -> CmdResult<()> {
    client.set_username(username.trim()).await.map_err(err)
}

/// Search users by username prefix.
#[tauri::command]
#[specta::specta]
pub async fn friend_search(
    client: State<'_, mc_core::server::ServerClient>,
    q: String,
) -> CmdResult<Vec<UserBrief>> {
    client.search_users(q.trim()).await.map_err(err)
}

/// Send a friend request by user id.
#[tauri::command]
#[specta::specta]
pub async fn friend_request(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_request(&user_id).await.map_err(err)
}

/// Accepted friends.
#[tauri::command]
#[specta::specta]
pub async fn friend_list(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<UserBrief>> {
    client.friends().await.map_err(err)
}

/// Incoming pending friend requests.
#[tauri::command]
#[specta::specta]
pub async fn friend_requests(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<UserBrief>> {
    client.friend_requests().await.map_err(err)
}

/// Accept a pending request from `user_id`.
#[tauri::command]
#[specta::specta]
pub async fn friend_accept(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_accept(&user_id).await.map_err(err)
}

/// Decline a pending request from `user_id`.
#[tauri::command]
#[specta::specta]
pub async fn friend_decline(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_decline(&user_id).await.map_err(err)
}

/// Remove a friend.
#[tauri::command]
#[specta::specta]
pub async fn friend_remove(
    client: State<'_, mc_core::server::ServerClient>,
    user_id: String,
) -> CmdResult<()> {
    client.friend_remove(&user_id).await.map_err(err)
}

/// Heartbeat the current user's presence; `activity` = the running instance name
/// (what they're playing), or `None` when idle.
#[tauri::command]
#[specta::specta]
pub async fn presence_heartbeat(
    client: State<'_, mc_core::server::ServerClient>,
    activity: Option<String>,
) -> CmdResult<()> {
    client.presence_heartbeat(activity.as_deref()).await.map_err(err)
}

// --- notifications (typed inbox: friend requests/accepts + realm invites) -----

/// The current user's last 50 notifications (newest first).
#[tauri::command]
#[specta::specta]
pub async fn notifications(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<mc_core::notification::Notification>> {
    client.notifications().await.map_err(err)
}

/// Mark every notification as read (called when the bell dropdown opens).
#[tauri::command]
#[specta::specta]
pub async fn notifications_read(client: State<'_, mc_core::server::ServerClient>) -> CmdResult<()> {
    client.mark_notifications_read().await.map_err(err)
}

// --- account linking (bind Microsoft identity to the kobeMC user) ------------

use mc_core::account::Identity;

/// Bind a Microsoft identity to the current kobeMC user. `account_id` is the
/// selected Microsoft account's Minecraft profile UUID; `username` is its
/// gamertag/MC username at link time (informational).
#[tauri::command]
#[specta::specta]
pub async fn account_link_microsoft(
    client: State<'_, mc_core::server::ServerClient>,
    account_id: String,
    username: Option<String>,
) -> CmdResult<()> {
    client.link_microsoft(account_id.trim(), username).await.map_err(err)
}

/// List the current kobeMC user's linked identities.
#[tauri::command]
#[specta::specta]
pub async fn account_identities(
    client: State<'_, mc_core::server::ServerClient>,
) -> CmdResult<Vec<Identity>> {
    client.account_identities().await.map_err(err)
}

/// Unlink a provider (e.g. `microsoft`) from the current kobeMC user.
#[tauri::command]
#[specta::specta]
pub async fn account_unlink(
    client: State<'_, mc_core::server::ServerClient>,
    provider: String,
) -> CmdResult<()> {
    client.unlink_provider(provider.trim()).await.map_err(err)
}

