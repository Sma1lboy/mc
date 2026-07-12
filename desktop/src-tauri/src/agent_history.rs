//! Native owner for local agent conversation history.
//!
//! WebKit localStorage is an implementation detail of the current webview and
//! can move when the app identifier or dev launcher changes. The launcher owns
//! this SQLite file instead; the record payload remains the UI's opaque JSON.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

const CONVERSATION_LIMIT: usize = 50;
const LEGACY_WEBKIT_MIGRATION: &str = "legacy-webkit-localstorage-v1";

#[derive(Debug, Clone)]
pub struct AgentHistoryStore {
    path: PathBuf,
}

impl AgentHistoryStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let store = Self { path };
        store.initialize()?;
        Ok(store)
    }

    pub fn load_all(&self) -> Result<Vec<String>, String> {
        let conn = self.connection()?;
        let mut statement = conn
            .prepare(
                "SELECT record_json FROM agent_conversations
                 ORDER BY updated_at_ms DESC, id DESC",
            )
            .map_err(|error| error.to_string())?;
        let records = statement
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|error| error.to_string())?;
        Ok(records)
    }

    pub fn upsert(&self, id: &str, record_json: &str) -> Result<(), String> {
        let record: serde_json::Value =
            serde_json::from_str(record_json).map_err(|error| error.to_string())?;
        let record_id = record
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "agent conversation is missing id".to_string())?;
        if record_id != id {
            return Err("agent conversation id does not match payload".to_string());
        }
        if !record
            .get("messages")
            .is_some_and(serde_json::Value::is_array)
        {
            return Err("agent conversation is missing messages".to_string());
        }

        let title = record
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let updated_at_ms = record
            .get("updatedAt")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_default();
        let mut conn = self.connection()?;
        let transaction = conn.transaction().map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO agent_conversations (id, title, updated_at_ms, record_json)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title,
                   updated_at_ms = excluded.updated_at_ms,
                   record_json = excluded.record_json
                 WHERE excluded.updated_at_ms >= agent_conversations.updated_at_ms",
                params![id, title, updated_at_ms, record_json],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM agent_conversations
                 WHERE id IN (
                   SELECT id FROM agent_conversations
                   ORDER BY updated_at_ms DESC, id DESC
                   LIMIT -1 OFFSET ?1
                 )",
                [CONVERSATION_LIMIT as i64],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    /// Import a WebKit localStorage database without modifying it.
    pub fn import_webkit_database(&self, path: &Path) -> Result<usize, String> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| error.to_string())?;
        let value: Option<Vec<u8>> = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = ?1",
                ["mc-launcher.agentConversations"],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(value) = value else {
            return Ok(0);
        };
        let raw = decode_webkit_utf16(&value)?;
        let records: Vec<serde_json::Value> =
            serde_json::from_str(&raw).map_err(|error| error.to_string())?;
        let mut imported = 0;
        for record in records {
            let Some(id) = record.get("id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let json = serde_json::to_string(&record).map_err(|error| error.to_string())?;
            if self.upsert(id, &json).is_ok() {
                imported += 1;
            }
        }
        Ok(imported)
    }

    /// One-time compatibility import for conversations written before native
    /// storage existed. Failure is non-fatal: a missing or locked WebKit cache
    /// must never stop the launcher from opening.
    pub fn import_legacy_webkit_once(&self) -> Result<(), String> {
        if self.migration_complete()? {
            return Ok(());
        }
        #[cfg(target_os = "macos")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                let webkit = PathBuf::from(home).join("Library/WebKit");
                for container in ["mc-launcher-desktop", "com.sma1lboy.mclauncher"] {
                    let mut paths = Vec::new();
                    collect_localstorage_databases(&webkit.join(container), 7, &mut paths);
                    for path in paths {
                        let _ = self.import_webkit_database(&path);
                    }
                }
            }
        }
        self.mark_migration_complete()
    }

    fn initialize(&self) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS agent_conversations (
               id TEXT PRIMARY KEY NOT NULL,
               title TEXT NOT NULL,
               updated_at_ms INTEGER NOT NULL,
               record_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS agent_conversations_updated_idx
               ON agent_conversations (updated_at_ms DESC, id DESC);
             CREATE TABLE IF NOT EXISTS agent_history_meta (
               key TEXT PRIMARY KEY NOT NULL,
               value TEXT NOT NULL
             );",
        )
        .map_err(|error| error.to_string())
    }

    fn connection(&self) -> Result<Connection, String> {
        Connection::open(&self.path).map_err(|error| error.to_string())
    }

    fn migration_complete(&self) -> Result<bool, String> {
        let conn = self.connection()?;
        conn.query_row(
            "SELECT 1 FROM agent_history_meta WHERE key = ?1",
            [LEGACY_WEBKIT_MIGRATION],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(|error| error.to_string())
    }

    fn mark_migration_complete(&self) -> Result<(), String> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT OR REPLACE INTO agent_history_meta (key, value) VALUES (?1, ?2)",
            params![LEGACY_WEBKIT_MIGRATION, "complete"],
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    }
}

fn decode_webkit_utf16(bytes: &[u8]) -> Result<String, String> {
    let chunks = bytes.chunks_exact(2);
    if !chunks.remainder().is_empty() {
        return Err("WebKit localStorage value has invalid UTF-16 length".to_string());
    }
    String::from_utf16(
        &chunks
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect::<Vec<_>>(),
    )
    .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
fn collect_localstorage_databases(root: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|name| name == "localstorage.sqlite3")
        {
            found.push(path);
        } else if path.is_dir() {
            collect_localstorage_databases(&path, depth - 1, found);
        }
    }
}
