use super::*;

// Columns selected for a RealmSummary (realm joined to the asking member).
type SummaryRow = (String, String, String, String, Option<String>, Option<String>, i32, String, Option<String>);
// (user_id, username, role, synced_version, joined_at) for the members query.
type MemberRow = (String, Option<String>, String, i32, Option<String>);
const SUMMARY_COLS: &str = "r.id, r.code, r.name, r.owner_id, r.mc_version, r.loader, r.manifest_version, m.role, r.expires_at::text";

fn row_to_summary(r: SummaryRow) -> RealmSummary {
    RealmSummary {
        id: r.0,
        code: r.1,
        name: r.2,
        owner_id: r.3,
        mc_version: r.4,
        loader: r.5,
        manifest_version: r.6,
        role: r.7,
        expires_at: r.8,
    }
}

#[derive(Clone)]
pub struct RealmStore {
    pool: PgPool,
}

impl RealmStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Create a realm owned by `owner_id`; mints id + code, seeds manifest at
    /// version 1, and enrolls the owner as the `owner` member.
    pub async fn create(&self, owner_id: &str, req: &CreateRealmReq) -> anyhow::Result<RealmSummary> {
        let manifest_json = serde_json::to_string(&req.manifest)?;
        let expires = req.expires_in_secs.unwrap_or(0);
        let (id,): (String,) = sqlx::query_as(
            "INSERT INTO realms (id, code, name, owner_id, mc_version, loader, manifest, manifest_version, expires_at) \
             VALUES (gen_random_uuid()::text, upper(substr(md5(gen_random_uuid()::text), 1, 6)), \
                     $1, $2, $3, $4, $5::jsonb, 1, \
                     CASE WHEN $6 > 0 THEN NOW() + ($6 * INTERVAL '1 second') ELSE NULL END) \
             RETURNING id",
        )
        .bind(&req.name)
        .bind(owner_id)
        .bind(&req.manifest.mc_version)
        .bind(&req.manifest.loader)
        .bind(&manifest_json)
        .bind(expires)
        .fetch_one(&self.pool)
        .await?;

        sqlx::query("INSERT INTO realm_members (realm_id, user_id, role) VALUES ($1, $2, 'owner')")
            .bind(&id)
            .bind(owner_id)
            .execute(&self.pool)
            .await?;

        self.summary_for(&id, owner_id).await?.ok_or_else(|| anyhow::anyhow!("realm vanished after create"))
    }

    /// Join by code (if not expired); idempotent. Returns the summary, or `None`
    /// if the code is unknown/expired.
    pub async fn join(&self, user_id: &str, code: &str) -> anyhow::Result<Option<RealmSummary>> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM realms WHERE code = $1 AND (expires_at IS NULL OR expires_at > NOW())",
        )
        .bind(code)
        .fetch_optional(&self.pool)
        .await?;
        let Some((id,)) = row else { return Ok(None) };

        sqlx::query(
            "INSERT INTO realm_members (realm_id, user_id, role) VALUES ($1, $2, 'member') \
             ON CONFLICT (realm_id, user_id) DO NOTHING",
        )
        .bind(&id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        self.summary_for(&id, user_id).await
    }

    /// Insert `target_id` as a plain `member` (idempotent). The caller must
    /// authorize first. Errors (FK violation) ⇒ the target user doesn't exist.
    pub async fn add_member(&self, realm_id: &str, target_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO realm_members (realm_id, user_id, role) VALUES ($1, $2, 'member') \
             ON CONFLICT (realm_id, user_id) DO NOTHING",
        )
        .bind(realm_id)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Realm summary *for a member* (None if the user isn't a member).
    pub async fn summary_for(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<RealmSummary>> {
        let row: Option<SummaryRow> = sqlx::query_as(&format!(
            "SELECT {SUMMARY_COLS} FROM realms r JOIN realm_members m ON m.realm_id = r.id \
             WHERE r.id = $1 AND m.user_id = $2",
        ))
        .bind(realm_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_summary))
    }

    /// Realms the user belongs to (owned or joined), newest first.
    pub async fn list_mine(&self, user_id: &str) -> anyhow::Result<Vec<RealmSummary>> {
        let rows: Vec<SummaryRow> = sqlx::query_as(&format!(
            "SELECT {SUMMARY_COLS} FROM realms r JOIN realm_members m ON m.realm_id = r.id \
             WHERE m.user_id = $1 ORDER BY r.created_at DESC",
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(row_to_summary).collect())
    }

    /// Current manifest + version for a member (None if not a member).
    pub async fn manifest(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<RealmManifest>> {
        let row: Option<(String, i32)> = sqlx::query_as(
            "SELECT r.manifest::text, r.manifest_version FROM realms r \
             JOIN realm_members m ON m.realm_id = r.id WHERE r.id = $1 AND m.user_id = $2",
        )
        .bind(realm_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some((json, version)) = row else { return Ok(None) };
        let mut manifest: RealmManifest = serde_json::from_str(&json).unwrap_or_default();
        manifest.version = version;
        Ok(Some(manifest))
    }

    /// Replace the manifest + bump version. Requires `owner`/`admin`. Returns
    /// the new version, or `None` if not permitted / realm gone.
    pub async fn push_manifest(
        &self,
        realm_id: &str,
        actor_id: &str,
        manifest: &RealmManifest,
    ) -> anyhow::Result<Option<i32>> {
        let json = serde_json::to_string(manifest)?;
        let row: Option<(i32,)> = sqlx::query_as(
            "UPDATE realms SET manifest = $3::jsonb, manifest_version = manifest_version + 1, \
                    mc_version = $4, loader = $5 \
             WHERE id = $1 AND EXISTS ( \
                 SELECT 1 FROM realm_members WHERE realm_id = $1 AND user_id = $2 AND role IN ('owner','admin')) \
             RETURNING manifest_version",
        )
        .bind(realm_id)
        .bind(actor_id)
        .bind(&json)
        .bind(&manifest.mc_version)
        .bind(&manifest.loader)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(v,)| v))
    }

    /// Member list for a realm (only if `user_id` is a member).
    pub async fn members(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<Vec<MemberInfo>>> {
        if self.role_of(realm_id, user_id).await?.is_none() {
            return Ok(None);
        }
        let rows: Vec<MemberRow> = sqlx::query_as(
            "SELECT m.user_id, u.username, m.role, m.synced_version, m.joined_at::text \
             FROM realm_members m JOIN users u ON u.id = m.user_id \
             WHERE m.realm_id = $1 ORDER BY m.joined_at",
        )
        .bind(realm_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(Some(
            rows.into_iter()
                .map(|(user_id, username, role, synced_version, joined_at)| MemberInfo {
                    user_id,
                    username,
                    role,
                    synced_version,
                    joined_at,
                })
                .collect(),
        ))
    }

    /// Owner sets another member's role (`admin`/`member`); never touches the
    /// owner row. Returns whether a row changed.
    pub async fn set_role(
        &self,
        realm_id: &str,
        actor_id: &str,
        target_id: &str,
        role: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "UPDATE realm_members SET role = $3 \
             WHERE realm_id = $1 AND user_id = $4 AND role <> 'owner' AND EXISTS ( \
                 SELECT 1 FROM realm_members WHERE realm_id = $1 AND user_id = $2 AND role = 'owner')",
        )
        .bind(realm_id)
        .bind(actor_id)
        .bind(role)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Self-leave, or owner removes a member. The owner can't leave (disband
    /// instead). Returns whether a row was removed.
    pub async fn leave_or_remove(
        &self,
        realm_id: &str,
        actor_id: &str,
        target_id: &str,
    ) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "DELETE FROM realm_members WHERE realm_id = $1 AND user_id = $3 AND role <> 'owner' AND ( \
                 $2 = $3 OR EXISTS (SELECT 1 FROM realm_members WHERE realm_id = $1 AND user_id = $2 AND role = 'owner'))",
        )
        .bind(realm_id)
        .bind(actor_id)
        .bind(target_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    /// Member records the version it has synced to (lets the owner see progress).
    pub async fn mark_synced(&self, realm_id: &str, user_id: &str, version: i32) -> anyhow::Result<()> {
        sqlx::query("UPDATE realm_members SET synced_version = $3 WHERE realm_id = $1 AND user_id = $2")
            .bind(realm_id)
            .bind(user_id)
            .bind(version)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Owner disbands the realm (cascades members). Returns whether it existed.
    pub async fn delete(&self, realm_id: &str, actor_id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query("DELETE FROM realms WHERE id = $1 AND owner_id = $2")
            .bind(realm_id)
            .bind(actor_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    async fn role_of(&self, realm_id: &str, user_id: &str) -> anyhow::Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT role FROM realm_members WHERE realm_id = $1 AND user_id = $2")
                .bind(realm_id)
                .bind(user_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(r,)| r))
    }

    /// Whether `user_id` may push content to the realm (owner/admin).
    pub async fn can_push(&self, realm_id: &str, user_id: &str) -> anyhow::Result<bool> {
        Ok(matches!(self.role_of(realm_id, user_id).await?.as_deref(), Some("owner") | Some("admin")))
    }

    /// Whether `user_id` is a member of the realm (can read content).
    pub async fn is_member(&self, realm_id: &str, user_id: &str) -> anyhow::Result<bool> {
        Ok(self.role_of(realm_id, user_id).await?.is_some())
    }

    /// Publish (or heartbeat) `user_id` as the realm's host, reachable at `address`.
    /// Caller must authorize membership first. Overwrites any previous host
    /// (last writer wins) and stamps `host_at = NOW()` as the heartbeat.
    pub async fn set_host(&self, realm_id: &str, user_id: &str, address: &str) -> anyhow::Result<()> {
        sqlx::query(
            "UPDATE realms SET host_address = $3, host_user_id = $2, host_at = NOW() WHERE id = $1",
        )
        .bind(realm_id)
        .bind(user_id)
        .bind(address)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The realm's *fresh* host (heartbeat within the last 90s) + its username,
    /// or `(None, None)` if nobody is hosting / the heartbeat went stale.
    pub async fn get_host(&self, realm_id: &str) -> anyhow::Result<(Option<String>, Option<String>)> {
        let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT r.host_address, u.username \
             FROM realms r LEFT JOIN users u ON u.id = r.host_user_id \
             WHERE r.id = $1 AND r.host_at IS NOT NULL AND r.host_at > NOW() - INTERVAL '90 seconds'",
        )
        .bind(realm_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.unwrap_or((None, None)))
    }

    /// Stop hosting: clear the host record **iff** `user_id` is the current host.
    /// Returns whether a row changed.
    pub async fn clear_host(&self, realm_id: &str, user_id: &str) -> anyhow::Result<bool> {
        let res = sqlx::query(
            "UPDATE realms SET host_address = NULL, host_user_id = NULL, host_at = NULL \
             WHERE id = $1 AND host_user_id = $2",
        )
        .bind(realm_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }
}

/* ---- handlers ---- */
