//! Local agent-runtime detection.
//!
//! The desktop can run the modpack agent on the user's locally-installed
//! Claude Code (subscription login, no API key) via a Node host process. That
//! path needs two things on this machine: the `claude` CLI (as the proxy for
//! "the user has a Claude Code login") and `node` + `pnpm` (the harness bridge
//! bootstraps itself with them). This module answers "is that available?" for
//! the settings UI — it never spawns the runtime itself.

use std::path::PathBuf;
use std::process::Command;

/// One detected executable: absolute path + `--version` output (trimmed).
#[derive(Debug, Clone)]
pub struct DetectedBinary {
    pub path: String,
    pub version: String,
}

/// Availability of the local Claude Code runtime path.
#[derive(Debug, Clone)]
pub struct LocalRuntimeStatus {
    /// The `claude` CLI, when installed (login-state proxy).
    pub claude_code: Option<DetectedBinary>,
    /// The `node` runtime the host process needs.
    pub node: Option<DetectedBinary>,
    /// `pnpm`, needed once by the harness bridge bootstrap.
    pub pnpm: Option<DetectedBinary>,
}

/// Detect everything the local Claude Code agent path needs.
pub fn detect_local_runtime() -> LocalRuntimeStatus {
    LocalRuntimeStatus {
        claude_code: detect("claude"),
        node: detect("node"),
        pnpm: detect("pnpm"),
    }
}

/// Find `name` on PATH or in the usual install locations (GUI apps on macOS
/// get a minimal PATH without /opt/homebrew/bin, so PATH alone is not enough),
/// then read its `--version`.
fn detect(name: &str) -> Option<DetectedBinary> {
    let path = find_binary(name)?;
    let out = Command::new(&path).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(DetectedBinary {
        path: path.to_string_lossy().into_owned(),
        version,
    })
}

fn find_binary(name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(path_var) = std::env::var("PATH") {
        candidates.extend(std::env::split_paths(&path_var).map(|d| d.join(name)));
    }
    let home = std::env::var("HOME").ok().map(PathBuf::from);
    for dir in [
        Some(PathBuf::from("/opt/homebrew/bin")),
        Some(PathBuf::from("/usr/local/bin")),
        home.as_ref().map(|h| h.join(".local/bin")),
        home.as_ref().map(|h| h.join(".claude/local")),
    ]
    .into_iter()
    .flatten()
    {
        candidates.push(dir.join(name));
    }
    candidates.into_iter().find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Why this test matters: `find_binary` is the only pure-ish seam — it must
    // find something that exists on every machine ("sh" on unix) and must not
    // false-positive on nonsense names. Version spawning is exercised for real
    // through the command layer.
    #[test]
    #[cfg(unix)]
    fn finds_sh_and_rejects_garbage() {
        assert!(find_binary("sh").is_some());
        assert!(find_binary("definitely-not-a-real-binary-xyz").is_none());
    }
}
