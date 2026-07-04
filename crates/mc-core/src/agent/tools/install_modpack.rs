//! `install_modpack` — import a `.mrpack` that `build_modpack` just wrote into a
//! real launcher instance, closing the "built a file but nothing plays it" gap.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::modpack::import::{ImportEngine, ImportOptions, ImportSource};

use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InstallModpackArgs {
    /// The `output_path` returned by a successful `build_modpack` in this
    /// conversation, verbatim. Only files inside the agent build output
    /// directory are accepted.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InstallBlockedFile {
    pub name: String,
    pub website_url: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct InstallModpackOutput {
    pub instance_id: String,
    /// CurseForge files the user must download manually (none for pure-Modrinth packs).
    pub blocked: Vec<InstallBlockedFile>,
    pub skipped_optional: Vec<String>,
}

/// Import an agent-built `.mrpack` into `dest_root` as a launcher instance.
///
/// Trust boundary: the model supplies `path`, so it is canonicalized and must
/// live inside `ctx.output_dir` — the sandbox `build_modpack` writes into. The
/// engine (downloader + registry) is injected by the host like every other
/// import path.
pub async fn tool_install_modpack(
    ctx: &ChatToolsCtx,
    engine: &ImportEngine,
    dest_root: &Path,
    args: InstallModpackArgs,
) -> Result<InstallModpackOutput, ChatToolError> {
    let requested = PathBuf::from(args.path.trim());
    let canon = requested
        .canonicalize()
        .map_err(|e| ChatToolError::new(format!("modpack file not found: {e}")))?;
    let sandbox = ctx
        .output_dir
        .canonicalize()
        .map_err(|e| ChatToolError::new(format!("agent output dir unavailable: {e}")))?;
    if !canon.starts_with(&sandbox) {
        return Err(ChatToolError::new(
            "path is outside the agent build output directory; only packs built in this conversation can be installed",
        ));
    }

    let outcome = engine
        .import(ImportSource::LocalFile(canon), ImportOptions::new(dest_root.to_path_buf()))
        .await
        .map_err(|e| ChatToolError::new(e.to_string()))?;

    Ok(InstallModpackOutput {
        instance_id: outcome.instance_id,
        blocked: outcome
            .blocked
            .into_iter()
            .map(|b| InstallBlockedFile { name: b.name, website_url: b.website_url, required: b.required })
            .collect(),
        skipped_optional: outcome.skipped_optional,
    })
}
