//! Account-linking client for `mc-server`: bind extra identities (notably
//! **Microsoft**) to the current kobeMC user, on top of better-auth's own
//! email/password identity. Mirrors the `/v1/account/*` endpoints. The session
//! lives on the held [`ServerClient`](crate::server::ServerClient).
//!
//! The launcher completes the real Microsoft device-code + Xbox/Minecraft flow
//! locally; once the user is in a kobeMC session it posts the verified MS
//! identity here (the Minecraft profile UUID) and the server records the link.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::server::ServerClient;

/// One linked identity of the current kobeMC user (e.g. `credential` email, or
/// `microsoft`). Mirrors `mc-server`'s `Identity`.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct Identity {
    pub provider: String,
    pub account_id: String,
}

/// Body for `POST /v1/account/link/microsoft` (mirrors `LinkMicrosoftReq`).
#[derive(Serialize)]
struct LinkMicrosoftBody {
    /// The Minecraft profile UUID (stable per Microsoft account).
    account_id: String,
    /// Display gamertag / MC username at link time (optional, informational).
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
}

impl ServerClient {
    /// Bind a Microsoft identity to the current kobeMC user. `account_id` is the
    /// Minecraft profile UUID (the stable per-Microsoft-account id, sourced from
    /// the selected Microsoft account); `username` is the gamertag/MC username at
    /// link time (informational only on the server).
    pub async fn link_microsoft(&self, account_id: &str, username: Option<String>) -> Result<()> {
        self.post_no_content(
            "/v1/account/link/microsoft",
            &LinkMicrosoftBody { account_id: account_id.to_string(), username },
        )
        .await
    }

    /// List the current user's linked identities.
    pub async fn account_identities(&self) -> Result<Vec<Identity>> {
        self.get_json("/v1/account/identities").await
    }

    /// Unlink a provider (e.g. `microsoft`) from the current user.
    pub async fn unlink_provider(&self, provider: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/account/link/{provider}")).await
    }
}
