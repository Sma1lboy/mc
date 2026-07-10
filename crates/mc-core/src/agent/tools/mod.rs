//! The deterministic tools the TS agent brain can call (via `agent_tool_*`).
//!
//! Each tool is a thin wrapper over an existing, tested `mc-core` primitive:
//! provider search, the dependency resolver, the base-modlist parser, and the
//! `.mrpack` executor. The tools take strictly-typed args and return structured
//! JSON built ONLY from real provider/resolver data — the model can never
//! fabricate project ids, version ids, urls, hashes, or filenames, because
//! those fields are always echoed straight from a provider call.
//!
//! `build_modpack` is the only tool that writes to disk, and it re-resolves every
//! file reference through the provider (`get_files_bulk`) rather than trusting
//! anything the model passed in.

mod build_modpack;
mod diagnose_instance;
mod inspect_base_modpack;
mod install_modpack;
mod list_instances;
mod mod_get_detail;
mod resolve_mods;
mod search_base_modpacks;
mod search_mods;
mod wiki;

#[cfg(test)]
mod fake_provider;
#[cfg(test)]
mod tests;

pub use build_modpack::*;
#[cfg(test)]
pub(crate) use diagnose_instance::diagnose_instance_with_total_memory;
pub use diagnose_instance::{
    tool_diagnose_instance, DiagnoseInstanceArgs, DiagnoseInstanceOutput, InstanceDiagnosticSummary,
};
pub use inspect_base_modpack::*;
pub use install_modpack::*;
pub use list_instances::*;
pub use mod_get_detail::*;
pub use resolve_mods::*;
pub use search_base_modpacks::*;
pub use search_mods::*;
pub use wiki::*;

use std::path::PathBuf;
use std::sync::Arc;

use crate::modplatform::provider::ProviderRegistry;
use crate::modplatform::ProviderId;

/// Error surfaced by a chat tool. Wraps any `mc-core` failure as a string so the
/// model sees a readable message and can adapt (retry, ask the user, etc.).
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ChatToolError(pub String);

impl ChatToolError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl From<crate::error::CoreError> for ChatToolError {
    fn from(err: crate::error::CoreError) -> Self {
        Self(err.to_string())
    }
}

/// Shared context injected into every tool: the provider registry to query and
/// the sandbox directory `build_modpack` writes finished `.mrpack` files into.
#[derive(Clone)]
pub struct ChatToolsCtx {
    /// Content-provider registry (Modrinth always; CurseForge when keyed).
    /// Injected, per the registry-injection convention, so tests use a fake.
    pub registry: Arc<ProviderRegistry>,
    /// Directory `build_modpack` writes into. The model supplies only a
    /// filename; the tool joins it here after sanitizing, so the model can never
    /// choose an arbitrary absolute path.
    pub output_dir: PathBuf,
}

impl ChatToolsCtx {
    pub fn new(registry: Arc<ProviderRegistry>, output_dir: impl Into<PathBuf>) -> Self {
        Self {
            registry,
            output_dir: output_dir.into(),
        }
    }
}

pub(super) fn provider_slug(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Modrinth => "modrinth",
        ProviderId::CurseForge => "curseforge",
    }
}

pub(super) fn provider_from_slug(slug: &str) -> ProviderId {
    match slug.trim().to_ascii_lowercase().as_str() {
        "curseforge" => ProviderId::CurseForge,
        _ => ProviderId::Modrinth,
    }
}
