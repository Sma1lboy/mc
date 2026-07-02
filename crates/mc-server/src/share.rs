//! Instance/modpack sharing, persisted in the `shares` table (sqlx). A user
//! publishes an instance's metadata + file manifest and gets a short content-
//! derived id others fetch and rebuild from.

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

/// One declared file in a shared instance (downloaded by url, verified by sha1).
#[derive(Serialize, Deserialize, Clone)]
pub struct SharedFile {
    pub path: String,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

/// A shared instance: enough to recreate it elsewhere.
#[derive(Serialize, Deserialize, Clone)]
pub struct SharedInstance {
    pub name: String,
    pub mc_version: String,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub loader_version: Option<String>,
    #[serde(default)]
    pub files: Vec<SharedFile>,
    /// Server-assigned; ignored on submit.
    #[serde(default)]
    pub id: String,
}

/// DB-backed share registry. Ids are derived deterministically from content so
/// resubmitting the same instance returns the same id (idempotent).
#[derive(Clone)]
pub struct ShareStore {
    pool: PgPool,
}

impl ShareStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn put(&self, mut inst: SharedInstance, user_id: &str) -> anyhow::Result<String> {
        let id = derive_id(&inst);
        inst.id = id.clone();
        let json = serde_json::to_string(&inst)?;
        self.insert(&id, &json, user_id).await?;
        Ok(id)
    }

    /// Shared upsert: content-derived id ⇒ resubmits are idempotent; the last
    /// publisher becomes the recorded owner.
    async fn insert(&self, id: &str, json: &str, user_id: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO shares (id, json, user_id) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET json = EXCLUDED.json, user_id = EXCLUDED.user_id",
        )
        .bind(id)
        .bind(json)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Option<SharedInstance> {
        let row: Option<(String,)> = sqlx::query_as("SELECT json FROM shares WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
        row.and_then(|(json,)| serde_json::from_str(&json).ok())
    }

    /// Store an opaque JSON blob (e.g. an agent chat transcript) under a
    /// content-derived id, reusing the same `shares` table. Idempotent: the same
    /// payload yields the same id. Used for public conversation sharing.
    pub async fn put_raw(&self, value: &serde_json::Value, user_id: &str) -> anyhow::Result<String> {
        let json = serde_json::to_string(value)?;
        let id = derive_raw_id(&json);
        self.insert(&id, &json, user_id).await?;
        Ok(id)
    }

    /// Fetch a raw JSON blob previously stored with [`put_raw`].
    pub async fn get_raw(&self, id: &str) -> Option<serde_json::Value> {
        let row: Option<(String,)> = sqlx::query_as("SELECT json FROM shares WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .ok()
            .flatten();
        row.and_then(|(json,)| serde_json::from_str(&json).ok())
    }
}

/// Content-derived id for a raw blob (same FNV-1a scheme as `derive_id`, over
/// the serialized JSON). Prefixed `c` so conversation ids don't collide with
/// instance ids in the shared table.
fn derive_raw_id(json: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in json.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("c{h:016x}")
}

/// Short stable id from the instance's defining fields (name+version+files).
/// A tiny FNV-1a hash keeps it dependency-free.
fn derive_id(inst: &SharedInstance) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |s: &str| {
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    feed(&inst.name);
    feed(&inst.mc_version);
    feed(inst.loader.as_deref().unwrap_or(""));
    for f in &inst.files {
        feed(&f.path);
        feed(&f.url);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SharedInstance {
        SharedInstance {
            name: "Pack".into(),
            mc_version: "1.20.1".into(),
            loader: Some("fabric".into()),
            loader_version: None,
            files: vec![],
            id: String::new(),
        }
    }

    #[tokio::test]
    async fn put_get_roundtrip_and_stable_id() {
        let Some(pool) = crate::db::test_pool().await else { return };
        // Publisher must exist (shares.user_id FK).
        sqlx::query("INSERT INTO users (id, name) VALUES ('share-test-user', 'tester') ON CONFLICT (id) DO NOTHING")
            .execute(&pool)
            .await
            .unwrap();
        let store = ShareStore::new(pool);
        let id1 = store.put(sample(), "share-test-user").await.unwrap();
        let id2 = store.put(sample(), "share-test-user").await.unwrap();
        assert_eq!(id1, id2); // deterministic / idempotent
        assert_eq!(store.get(&id1).await.unwrap().name, "Pack");
        assert!(store.get("nonexistent").await.is_none());
    }
}
