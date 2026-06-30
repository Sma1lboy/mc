//! Local daemon-owned agent session storage.
//!
//! This is deliberately file-backed and small. The daemon owns session ids and
//! persisted snapshots; UI clients should keep only the `session_id`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoResultExt, Result};

use super::state::{AgentPhase, AgentRunSnapshot, AgentStatus, ApprovalKind};

const SNAPSHOT_FILE: &str = "snapshot.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSessionSummary {
    pub session_id: String,
    pub status: AgentStatus,
    pub phase: AgentPhase,
    pub user_prompt: String,
    pub updated_at_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval_kind: Option<ApprovalKind>,
}

#[derive(Debug, Clone)]
pub struct AgentSessionStore {
    root: PathBuf,
}

impl AgentSessionStore {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self {
            root: data_dir.as_ref().join("agent").join("sessions"),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn save_snapshot(&self, snapshot: &AgentRunSnapshot) -> Result<PathBuf> {
        validate_session_id(&snapshot.id)?;
        let path = self.snapshot_path(&snapshot.id)?;
        let raw = serde_json::to_vec_pretty(snapshot).map_err(|e| CoreError::Parse {
            what: "agent session snapshot".into(),
            source: e,
        })?;
        crate::fs::write_atomic(&path, &raw)?;
        Ok(path)
    }

    pub fn load_snapshot(&self, session_id: &str) -> Result<AgentRunSnapshot> {
        let path = self.snapshot_path(session_id)?;
        let raw = std::fs::read(&path).with_path(&path)?;
        serde_json::from_slice(&raw).map_err(|e| CoreError::Parse {
            what: "agent session snapshot".into(),
            source: e,
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<AgentSessionSummary>> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }

        let mut out = Vec::new();
        for entry in std::fs::read_dir(&self.root).with_path(&self.root)? {
            let entry = entry.with_path(&self.root)?;
            if !entry.file_type().with_path(entry.path())?.is_dir() {
                continue;
            }
            let Some(session_id) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if !is_valid_session_id(&session_id) {
                continue;
            }
            let snapshot_path = entry.path().join(SNAPSHOT_FILE);
            if !snapshot_path.exists() {
                continue;
            }
            let snapshot = self.load_snapshot(&session_id)?;
            out.push(summary_from_snapshot(&snapshot));
        }
        out.sort_by_key(|entry| std::cmp::Reverse(entry.updated_at_ms));
        Ok(out)
    }

    pub fn delete_session(&self, session_id: &str) -> Result<bool> {
        let dir = self.session_dir(session_id)?;
        if !dir.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&dir).with_path(&dir)?;
        Ok(true)
    }

    fn snapshot_path(&self, session_id: &str) -> Result<PathBuf> {
        Ok(self.session_dir(session_id)?.join(SNAPSHOT_FILE))
    }

    fn session_dir(&self, session_id: &str) -> Result<PathBuf> {
        validate_session_id(session_id)?;
        Ok(self.root.join(session_id))
    }
}

fn summary_from_snapshot(snapshot: &AgentRunSnapshot) -> AgentSessionSummary {
    AgentSessionSummary {
        session_id: snapshot.id.clone(),
        status: snapshot.status.clone(),
        phase: snapshot.phase.clone(),
        user_prompt: snapshot.user_prompt.clone(),
        updated_at_ms: snapshot.trace.last().map(|t| t.at_ms).unwrap_or(0),
        pending_approval_kind: snapshot.pending_approval.as_ref().map(|a| a.kind.clone()),
    }
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if is_valid_session_id(session_id) {
        Ok(())
    } else {
        Err(CoreError::other(format!(
            "invalid agent session id: {session_id}"
        )))
    }
}

fn is_valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::{AgentMessageKind, AgentRunSnapshot};

    fn temp_data_dir() -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        dir.push(format!(
            "mc-core-agent-session-test-{}-{}",
            std::process::id(),
            nanos
        ));
        dir
    }

    #[test]
    fn saves_loads_lists_and_deletes_snapshot() {
        let dir = temp_data_dir();
        let store = AgentSessionStore::new(&dir);
        let mut snapshot = AgentRunSnapshot::new("make an aviation colony pack");
        snapshot.id = "agent-run-test".to_string();
        snapshot.push_message(AgentMessageKind::User, "hello");
        snapshot.push_trace("created");

        let path = store.save_snapshot(&snapshot).unwrap();
        assert!(path.ends_with("snapshot.json"));

        let loaded = store.load_snapshot("agent-run-test").unwrap();
        assert_eq!(loaded.id, "agent-run-test");
        assert_eq!(loaded.messages.len(), 1);

        let listed = store.list_sessions().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "agent-run-test");

        assert!(store.delete_session("agent-run-test").unwrap());
        assert!(!store.delete_session("agent-run-test").unwrap());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_path_like_session_ids() {
        let store = AgentSessionStore::new(temp_data_dir());
        assert!(store.load_snapshot("../nope").is_err());
        assert!(store.load_snapshot("a/b").is_err());
    }
}
