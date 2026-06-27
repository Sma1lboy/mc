//! Friends client for `mc-server`: username search + request/accept. Mirrors the
//! `/v1/friends/*` + `/v1/users/search` + `/v1/account/username` endpoints. The
//! session lives on the held [`ServerClient`](crate::server::ServerClient).

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::server::ServerClient;

/// A minimal public view of a user (search result / friend / pending request).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct UserBrief {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
}

#[derive(Serialize)]
struct UserIdBody {
    user_id: String,
}
#[derive(Serialize)]
struct UsernameBody {
    username: String,
}

impl ServerClient {
    /// Set the current user's username (needed before friend search works).
    pub async fn set_username(&self, username: &str) -> Result<()> {
        self.post_no_content("/v1/account/username", &UsernameBody { username: username.to_string() }).await
    }

    /// Search users by username prefix.
    pub async fn search_users(&self, q: &str) -> Result<Vec<UserBrief>> {
        self.get_json_query("/v1/users/search", &[("q", q)]).await
    }

    /// Send a friend request (auto-accepts a reverse pending request).
    pub async fn friend_request(&self, user_id: &str) -> Result<()> {
        self.post_no_content("/v1/friends/request", &UserIdBody { user_id: user_id.to_string() }).await
    }

    /// Accepted friends.
    pub async fn friends(&self) -> Result<Vec<UserBrief>> {
        self.get_json("/v1/friends").await
    }

    /// Incoming pending friend requests.
    pub async fn friend_requests(&self) -> Result<Vec<UserBrief>> {
        self.get_json("/v1/friends/requests").await
    }

    /// Accept a pending request from `user_id`.
    pub async fn friend_accept(&self, user_id: &str) -> Result<()> {
        self.post_no_content("/v1/friends/accept", &UserIdBody { user_id: user_id.to_string() }).await
    }

    /// Decline a pending request from `user_id`.
    pub async fn friend_decline(&self, user_id: &str) -> Result<()> {
        self.post_no_content("/v1/friends/decline", &UserIdBody { user_id: user_id.to_string() }).await
    }

    /// Remove a friend.
    pub async fn friend_remove(&self, user_id: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/friends/{user_id}")).await
    }
}
