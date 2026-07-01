use super::*;

#[derive(Clone)]
pub(in crate::agent::workflow) struct BasePackCandidate {
    pub(in crate::agent::workflow) provider: ProviderId,
    pub(in crate::agent::workflow) hit: SearchHit,
    pub(in crate::agent::workflow) matched_query: String,
    pub(in crate::agent::workflow) resolved_target: Option<TargetCompatibility>,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct SelectedBasePack {
    pub(in crate::agent::workflow) provider: ProviderId,
    pub(in crate::agent::workflow) project_id: String,
    pub(in crate::agent::workflow) slug: String,
    pub(in crate::agent::workflow) title: String,
    pub(in crate::agent::workflow) description: Option<String>,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct ModCandidate {
    pub(in crate::agent::workflow) provider: ProviderId,
    pub(in crate::agent::workflow) hit: SearchHit,
    pub(in crate::agent::workflow) matched_query: String,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct ResolvedModCandidate {
    pub(in crate::agent::workflow) candidate: ModCandidate,
    pub(in crate::agent::workflow) version: crate::modplatform::ProjectVersion,
    pub(in crate::agent::workflow) file: VersionFile,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct RequestedCompatibility {
    pub(in crate::agent::workflow) minecraft_version: Option<String>,
    pub(in crate::agent::workflow) loader: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct GeneratedRestrictionUpdate {
    pub(in crate::agent::workflow) input: UpdateBuildRestrictionsInput,
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct GeneratedModSearchPlan {
    pub(in crate::agent::workflow) queries: Vec<String>,
    pub(in crate::agent::workflow) retain_existing_mods: bool,
    pub(in crate::agent::workflow) remove_existing_mod_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::agent::workflow) enum BaseSearchMode {
    Strict,
    Loose,
    Tight,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct BaseModlistCache {
    pub(in crate::agent::workflow) refs: Vec<ModRef>,
    pub(in crate::agent::workflow) source_format: String,
    pub(in crate::agent::workflow) fetch_count: u32,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct ValidatedCustomizationPlan {
    pub(in crate::agent::workflow) extra_mods: Vec<serde_json::Value>,
    pub(in crate::agent::workflow) validation: serde_json::Value,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct CustomizationPlanningBlocked {
    pub(in crate::agent::workflow) reason: String,
    pub(in crate::agent::workflow) replan_phase: AgentPhase,
    pub(in crate::agent::workflow) details: serde_json::Value,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) enum CustomizationPlanningResult {
    Validated(ValidatedCustomizationPlan),
    Blocked(CustomizationPlanningBlocked),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::agent::workflow) enum ExecutionOutcomeKind {
    Ready,
    Verifying,
    Completed,
    Blocked,
    Retry,
    Failed,
}

#[derive(Debug, Clone)]
pub(in crate::agent::workflow) struct ExecutionOutcome {
    pub(in crate::agent::workflow) kind: ExecutionOutcomeKind,
    pub(in crate::agent::workflow) reason: Option<String>,
    pub(in crate::agent::workflow) replan_phase: Option<AgentPhase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::agent::workflow) enum ChangedField {
    MinecraftVersion,
    Loader,
    VersionRequirement,
    ContentPreference,
    SearchPreference,
    BasePack,
}
