//! `list_instances` — read-only view of the launcher's instances so the agent
//! knows what the user already has (versions, loaders) before proposing work.

use serde::{Deserialize, Serialize};

use crate::paths::GamePaths;

use super::ChatToolError;

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct AgentInstance {
    pub id: String,
    pub name: String,
    pub mc_version: String,
    pub loader: String,
    pub loader_version: Option<String>,
    /// Whether the core (version + loader) is installed and launchable.
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ListInstancesOutput {
    pub instances: Vec<AgentInstance>,
}

/// Lean instance list for the agent: identity + target only, no icons or
/// launch state (the full `InstanceSummary` base64-encodes icons — wasted
/// tokens for a model).
pub fn tool_list_instances(paths: &GamePaths) -> Result<ListInstancesOutput, ChatToolError> {
    let instances = crate::instance::list_instances(paths)
        .into_iter()
        .map(|s| AgentInstance {
            id: s.id,
            name: s.name,
            mc_version: s.mc_version,
            loader: s.loader.as_str().to_string(),
            loader_version: s.loader_version,
            installed: s.installed,
        })
        .collect();
    Ok(ListInstancesOutput { instances })
}
