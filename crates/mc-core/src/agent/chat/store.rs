//! Tiny file-backed persistence for chat transcripts, so a conversation
//! survives an app restart.
//!
//! One pretty-printed JSON file per session at
//! `<data_dir>/agent/chat/sessions/<session_id>.json`. Session ids are
//! validated with the same rule as [`crate::agent::session`] (alnum / `-` /
//! `_` only), so a caller-supplied id can never escape the sessions directory.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, IoResultExt, Result};

use super::run::ChatTranscript;

fn transcript_path(data_dir: &Path, session_id: &str) -> Result<PathBuf> {
    if !crate::agent::session::is_valid_session_id(session_id) {
        return Err(CoreError::other(format!(
            "invalid chat session id: {session_id}"
        )));
    }
    Ok(data_dir
        .join("agent")
        .join("chat")
        .join("sessions")
        .join(format!("{session_id}.json")))
}

/// Persist a session's transcript (atomic write, pretty JSON).
pub fn save_transcript(
    data_dir: &Path,
    session_id: &str,
    transcript: &ChatTranscript,
) -> Result<PathBuf> {
    let path = transcript_path(data_dir, session_id)?;
    let raw = serde_json::to_vec_pretty(transcript).map_err(|e| CoreError::Parse {
        what: "chat transcript".into(),
        source: e,
    })?;
    crate::fs::write_atomic(&path, &raw)?;
    Ok(path)
}

/// Load a session's persisted transcript, if any. A missing, invalid, or
/// unparseable file yields `None` (the conversation simply starts fresh) —
/// a stale transcript must never brick a session.
pub fn load_transcript(data_dir: &Path, session_id: &str) -> Option<ChatTranscript> {
    let path = transcript_path(data_dir, session_id).ok()?;
    let raw = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&raw) {
        Ok(transcript) => Some(transcript),
        Err(e) => {
            tracing::warn!(
                "ignoring unreadable chat transcript {}: {e}",
                path.display()
            );
            None
        }
    }
}

/// Delete a session's persisted transcript. Missing file is fine (already gone).
pub fn delete_transcript(data_dir: &Path, session_id: &str) -> Result<()> {
    let path = transcript_path(data_dir, session_id)?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_path(&path),
    }
}
