//! Notifications client for `mc-server`: a typed, server-backed inbox the
//! launcher polls (the top-bar bell). Mirrors the `/v1/notifications` endpoints.
//! The session lives on the held [`ServerClient`](crate::server::ServerClient).

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::server::ServerClient;

/// A notification addressed to the current user. `kind` is one of
/// `friend_request` | `friend_accepted` | `realm_invite`; `actor_*` is who
/// caused it and `realm_*` the realm it concerns (resolved server-side). `read`
/// is whether the user has already seen it (cleared on bell open).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct Notification {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub actor_username: Option<String>,
    #[serde(default)]
    pub realm_id: Option<String>,
    #[serde(default)]
    pub realm_name: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub read: bool,
}

impl ServerClient {
    /// The current user's last 50 notifications (newest first).
    pub async fn notifications(&self) -> Result<Vec<Notification>> {
        self.get_json("/v1/notifications").await
    }

    /// Mark every notification as read (called when the bell dropdown opens).
    pub async fn mark_notifications_read(&self) -> Result<()> {
        self.post_no_content("/v1/notifications/read", &serde_json::json!({})).await
    }
}
