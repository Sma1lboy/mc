use super::*;

/* ---------- client methods ---------- */

impl ServerClient {
    /// Create a realm (the caller must be logged in); returns the new summary.
    pub async fn create_realm(&self, req: &CreateRealmReq) -> Result<RealmSummary> {
        self.post_json("/v1/realms", req).await
    }

    /// Join by code. `Ok(None)` if the code is unknown/expired.
    pub async fn join_realm(&self, code: &str) -> Result<Option<RealmSummary>> {
        self.post_optional_json("/v1/realms/join", &JoinBody { code: code.to_string() }).await
    }

    /// Realms the current user belongs to (owned or joined), newest first.
    pub async fn list_realms(&self) -> Result<Vec<RealmSummary>> {
        self.get_json("/v1/realms/mine").await
    }

    /// A single realm's summary (for the current user).
    pub async fn get_realm(&self, id: &str) -> Result<RealmSummary> {
        self.get_json(&format!("/v1/realms/{id}")).await
    }

    /// The realm's current manifest + version.
    pub async fn realm_manifest(&self, id: &str) -> Result<RealmManifest> {
        self.get_json(&format!("/v1/realms/{id}/manifest")).await
    }

    /// Publish a new manifest (owner/admin only); returns the bumped version.
    pub async fn push_realm_manifest(&self, id: &str, manifest: &RealmManifest) -> Result<i32> {
        let r: VersionResp = self.post_json(&format!("/v1/realms/{id}/manifest"), manifest).await?;
        Ok(r.version)
    }

    /// Member list (only if the current user is a member).
    pub async fn realm_members(&self, id: &str) -> Result<Vec<RealmMember>> {
        self.get_json(&format!("/v1/realms/{id}/members")).await
    }

    /// Invite an accepted friend straight into the realm (owner/admin only; no
    /// join code needed). The target is added as a plain member.
    pub async fn realm_invite(&self, realm_id: &str, user_id: &str) -> Result<()> {
        self.post_no_content(
            &format!("/v1/realms/{realm_id}/invite"),
            &InviteBody { user_id: user_id.to_string() },
        )
        .await
    }

    /// Owner sets another member's role (`admin`/`member`).
    pub async fn set_member_role(&self, id: &str, uid: &str, role: &str) -> Result<()> {
        self.post_no_content(
            &format!("/v1/realms/{id}/members/{uid}/role"),
            &RoleBody { role: role.to_string() },
        )
        .await
    }

    /// Self-leave, or owner removes a member.
    pub async fn remove_member(&self, id: &str, uid: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/realms/{id}/members/{uid}")).await
    }

    /// Record the manifest version this member has synced to.
    pub async fn mark_realm_synced(&self, id: &str, version: i32) -> Result<()> {
        self.post_no_content(&format!("/v1/realms/{id}/synced"), &SyncedBody { version }).await
    }

    /// Owner disbands the realm.
    pub async fn disband_realm(&self, id: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/realms/{id}")).await
    }

    /// Upload the realm's overrides blob (owner/admin); paired with a manifest push.
    pub async fn upload_overrides(&self, id: &str, zip: Vec<u8>) -> Result<()> {
        self.post_bytes(&format!("/v1/realms/{id}/overrides"), zip).await
    }

    /// Download the realm's overrides blob (member).
    pub async fn download_overrides(&self, id: &str) -> Result<Vec<u8>> {
        self.get_bytes(&format!("/v1/realms/{id}/overrides")).await
    }

    /// Publish (or heartbeat) my reachable host address (`<virtual_ip>:<lan_port>`)
    /// for a realm, so members can one-click join my LAN-opened world. Acts as a
    /// heartbeat — call ~every 30s while hosting (the server expires it after 90s).
    pub async fn realm_set_host(&self, realm_id: &str, address: &str) -> Result<()> {
        self.post_no_content(
            &format!("/v1/realms/{realm_id}/host"),
            &SetHostBody { address: address.to_string() },
        )
        .await
    }

    /// Who (if anyone) is currently hosting a realm — only returns a *fresh*
    /// host (server-side TTL). `address`/`host_username` are `None` when nobody
    /// is hosting (or the last heartbeat went stale).
    pub async fn realm_get_host(&self, realm_id: &str) -> Result<RealmHost> {
        self.get_json(&format!("/v1/realms/{realm_id}/host")).await
    }

    /// Stop hosting (clears my host record). No-op if I wasn't the host.
    pub async fn realm_clear_host(&self, realm_id: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/realms/{realm_id}/host")).await
    }
}

/* ---------- syncer ---------- */
