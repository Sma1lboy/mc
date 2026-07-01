//! Serializable state for main-agent runs and subworkflow sessions.
//!
//! The launcher daemon remains the source of truth for game operations. The
//! agent can return approval gates, but it does not own installation or writes.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{CoreError, Result};
use crate::modplatform::{Dependency, VersionFile};

pub const AGENT_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const MAX_AGENT_MESSAGES: usize = 300;
const MAX_AGENT_TRACE_EVENTS: usize = 300;
const MAX_AGENT_REPLANS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Running,
    WaitingForUser,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkflowKind {
    MainRouting,
    #[default]
    ModpackBuild,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkflowId {
    BuildModpack,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEntry {
    #[default]
    Home,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLaunchContext {
    #[serde(default)]
    pub entry: AgentEntry,
    #[serde(default = "default_available_workflows")]
    pub available_workflows: Vec<AgentWorkflowId>,
}

impl Default for AgentLaunchContext {
    fn default() -> Self {
        Self::from_entry(AgentEntry::Home)
    }
}

impl AgentLaunchContext {
    pub fn from_entry(entry: AgentEntry) -> Self {
        let available_workflows = match &entry {
            AgentEntry::Home => vec![AgentWorkflowId::BuildModpack],
        };
        Self {
            entry,
            available_workflows,
        }
    }

    pub fn allows_workflow(&self, workflow: AgentWorkflowId) -> bool {
        self.available_workflows.contains(&workflow)
    }
}

fn default_available_workflows() -> Vec<AgentWorkflowId> {
    AgentLaunchContext::default().available_workflows
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    IntentExtraction,
    IntentRouting,
    ConfigureRequirementsApproval,
    BasePackSearch,
    BasePackRanking,
    ChooseBasePackApproval,
    CustomizationPlanning,
    ConfirmCustomizationApproval,
    ExecutionReady,
    Executing,
    Verifying,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentIntentKind {
    BuildModpack,
    Unknown,
}

impl AgentIntentKind {
    pub fn workflow_id(&self) -> Option<AgentWorkflowId> {
        match self {
            Self::BuildModpack => Some(AgentWorkflowId::BuildModpack),
            Self::Unknown => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIntent {
    pub kind: AgentIntentKind,
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    /// User confirms normalized target tags before any platform search.
    ConfigureRequirements,
    /// User picks one base modpack from ranked candidates.
    ChooseBasePack,
    /// User approves the extra mods and high-impact edits before execution.
    ConfirmCustomization,
    /// User explicitly accepts falling back to build-from-scratch mode.
    ConfirmScratchFallback,
    /// MVP-only gate: review an LLM draft before any real tool execution exists.
    ReviewDraftPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub kind: ApprovalKind,
    pub title: String,
    pub message: String,
    #[serde(default)]
    pub options: Vec<ApprovalOption>,
    #[serde(default)]
    pub available_decisions: Vec<ApprovalDecisionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AgentToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ModpackAgentPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolSpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub output_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalDecisionSpec {
    pub kind: UserDecisionKind,
    pub label: String,
    #[serde(default)]
    pub requires_selected_option: bool,
    #[serde(default)]
    pub requires_message: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Structured UI/daemon metadata for the selected option. Keep this JSON
    /// stable enough for IPC, but avoid making it the only source of truth.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserDecisionKind {
    Approve,
    Revise,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDecision {
    pub approval_id: String,
    pub kind: UserDecisionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Free-form structured edits reserved for future UI cards.
    #[serde(default)]
    pub edits: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunSnapshot {
    #[serde(default = "default_snapshot_schema_version")]
    pub schema_version: u32,
    pub id: String,
    #[serde(default)]
    pub workflow: AgentWorkflowKind,
    #[serde(default)]
    pub launch_context: AgentLaunchContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<AgentIntent>,
    pub status: AgentStatus,
    pub phase: AgentPhase,
    pub user_prompt: String,
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<ApprovalRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AgentToolSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ModpackAgentPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restrictions: Option<BuildRestrictions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_plan: Option<ModPlanState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_build: Option<ApprovedModpackBuild>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<AgentExecutionMetadata>,
    #[serde(default)]
    pub replans: Vec<PlanReplanRequest>,
    #[serde(default)]
    pub trace: Vec<AgentTraceEvent>,
}

impl AgentRunSnapshot {
    pub fn new(user_prompt: impl Into<String>) -> Self {
        Self {
            schema_version: AGENT_SNAPSHOT_SCHEMA_VERSION,
            id: new_id("agent-run"),
            workflow: AgentWorkflowKind::ModpackBuild,
            launch_context: AgentLaunchContext::default(),
            intent: None,
            status: AgentStatus::Running,
            phase: AgentPhase::IntentExtraction,
            user_prompt: user_prompt.into(),
            messages: Vec::new(),
            pending_approval: None,
            tools: Vec::new(),
            plan: None,
            restrictions: None,
            mod_plan: None,
            approved_build: None,
            execution: None,
            replans: Vec::new(),
            trace: Vec::new(),
        }
    }

    pub fn push_message(&mut self, kind: AgentMessageKind, text: impl Into<String>) {
        self.messages.push(AgentMessage {
            kind,
            text: text.into(),
        });
        self.trim_messages();
    }

    pub fn push_trace(&mut self, event: impl Into<String>) {
        self.trace.push(AgentTraceEvent {
            at_ms: now_ms(),
            event: event.into(),
            stage: None,
            iteration: None,
            tool: None,
            input: None,
            output: None,
            duration_ms: None,
            status: None,
        });
        self.trim_trace();
    }

    pub fn push_tool_trace(&mut self, trace: AgentToolTrace) {
        self.trace.push(AgentTraceEvent {
            at_ms: now_ms(),
            event: trace.event,
            stage: Some(trace.stage),
            iteration: Some(trace.iteration),
            tool: Some(trace.tool),
            input: Some(trace.input),
            output: Some(trace.output),
            duration_ms: Some(trace.duration_ms),
            status: Some(trace.status),
        });
        self.trim_trace();
    }

    pub fn push_replan(&mut self, request: PlanReplanRequest) {
        self.replans.push(request);
        self.trim_replans();
    }

    /// Enter a human-approval gate atomically: pause as [`AgentStatus::WaitingForUser`]
    /// at `phase`, holding `approval` and `plan`. This is the single home for the
    /// `WaitingForUser` ⇔ `pending_approval.is_some()` invariant; callers must not
    /// hand-set the status/phase/pending_approval trio. Secondary fields a gate may
    /// also touch (`tools`, and `approved_build`/`execution`/`mod_plan` invalidation)
    /// stay with the caller — they are deliberately not uniform across gates.
    pub fn request_approval(
        &mut self,
        phase: AgentPhase,
        approval: ApprovalRequest,
        plan: Option<ModpackAgentPlan>,
    ) {
        self.status = AgentStatus::WaitingForUser;
        self.phase = phase;
        self.pending_approval = Some(approval);
        self.plan = plan;
    }

    /// Leave any gate into a running `phase`, clearing the pending approval so the
    /// `Running` ⇒ no-pending-approval half of the invariant holds by construction.
    pub fn enter_phase(&mut self, phase: AgentPhase) {
        self.status = AgentStatus::Running;
        self.phase = phase;
        self.pending_approval = None;
    }

    /// Terminate the run as completed at `phase`, clearing any pending approval.
    pub fn complete(&mut self, phase: AgentPhase) {
        self.status = AgentStatus::Completed;
        self.phase = phase;
        self.pending_approval = None;
    }

    /// Terminate the run as failed at `phase`, clearing any pending approval.
    pub fn fail(&mut self, phase: AgentPhase) {
        self.status = AgentStatus::Failed;
        self.phase = phase;
        self.pending_approval = None;
    }

    fn trim_messages(&mut self) {
        if self.messages.len() <= MAX_AGENT_MESSAGES {
            return;
        }
        let keep_original_prompt = self.messages.first().is_some_and(|message| {
            message.kind == AgentMessageKind::User && message.text == self.user_prompt
        });
        while self.messages.len() > MAX_AGENT_MESSAGES {
            if keep_original_prompt && self.messages.len() > 1 {
                self.messages.remove(1);
            } else {
                self.messages.remove(0);
            }
        }
    }

    fn trim_trace(&mut self) {
        let excess = self.trace.len().saturating_sub(MAX_AGENT_TRACE_EVENTS);
        if excess > 0 {
            self.trace.drain(0..excess);
        }
    }

    fn trim_replans(&mut self) {
        let excess = self.replans.len().saturating_sub(MAX_AGENT_REPLANS);
        if excess > 0 {
            self.replans.drain(0..excess);
        }
    }
}

fn default_snapshot_schema_version() -> u32 {
    AGENT_SNAPSHOT_SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub kind: AgentMessageKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTraceEvent {
    pub at_ms: u128,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<AgentPhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AgentToolTrace {
    pub event: String,
    pub stage: AgentPhase,
    pub iteration: u32,
    pub tool: String,
    pub input: Value,
    pub output: Value,
    pub duration_ms: u128,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModpackAgentPlan {
    pub objective: String,
    /// Human-readable plan draft. Later phases can add typed candidates while
    /// preserving this explanation for transparency.
    pub summary_markdown: String,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub planned_actions: Vec<PlannedAction>,
    /// Notes that document where this Rust-native MVP can be migrated to a
    /// remote/sidecar orchestrator without changing daemon tool semantics.
    #[serde(default)]
    pub migration_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    pub id: String,
    pub label: String,
    /// Stable daemon tool name, e.g. "search_modpacks" or "install_mod".
    pub tool: String,
    #[serde(default)]
    pub args: serde_json::Value,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, JsonSchema)]
pub struct BuildRestrictions {
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version_requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(default)]
    pub feature_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default)]
    pub history: Vec<BuildRestrictionChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildRestrictionsLlmView {
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version_requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(default)]
    pub feature_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl BuildRestrictions {
    pub fn llm_view(&self) -> BuildRestrictionsLlmView {
        BuildRestrictionsLlmView {
            revision: self.revision,
            minecraft_version: self.minecraft_version.clone(),
            minecraft_version_requirement: self.minecraft_version_requirement.clone(),
            loader: self.loader.clone(),
            feature_tags: self.feature_tags.clone(),
            notes: self.notes.clone(),
        }
    }

    /// Apply a restriction patch under optimistic concurrency and return the
    /// resulting view.
    ///
    /// This is the single authority for mutating build restrictions. It rejects
    /// the write when `base_revision` no longer matches the current revision
    /// (same `Err` shape the caller relied on before), then runs ONE
    /// normalization pass: an invalid `minecraft_version` is dropped *with a
    /// warning* (not silently), the version requirement falls back to the
    /// concrete version, the loader is whitelisted (warning on an unsupported
    /// one), and feature tags are trimmed, lowercased, capped, then deduped.
    /// The normalized patch is stored, the revision is bumped, and a history
    /// entry is appended. `missing_fields`/`warnings` are derived from the
    /// stored result. The free `update_build_restrictions` wrapper and every
    /// replan route here, so the two normalization passes that used to drift
    /// can no longer disagree.
    pub(super) fn try_apply(
        &mut self,
        base_revision: u64,
        patch: BuildRestrictionPatch,
        source: BuildRestrictionChangeSource,
        summary: impl Into<String>,
    ) -> Result<UpdateBuildRestrictionsOutput> {
        if base_revision != self.revision {
            return Err(CoreError::other(format!(
                "update_build_restrictions revision mismatch: expected {}, got {}",
                self.revision, base_revision
            )));
        }

        let mut warnings = Vec::new();
        let minecraft_version = patch
            .minecraft_version
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| {
                if is_minecraft_version(s) {
                    Some(s.to_string())
                } else {
                    warnings.push(format!("ignored invalid minecraft_version: {s}"));
                    None
                }
            });
        let minecraft_version_requirement = patch
            .minecraft_version_requirement
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| minecraft_version.clone());
        let loader = patch.loader.as_deref().and_then(normalize_loader);
        if patch.loader.is_some() && loader.is_none() {
            warnings.push("ignored unsupported loader".to_string());
        }
        let notes = patch
            .notes
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);
        let normalized = BuildRestrictionPatch {
            minecraft_version,
            minecraft_version_requirement,
            loader,
            feature_tags: normalize_feature_tags(patch.feature_tags),
            notes,
        };

        self.minecraft_version = normalized.minecraft_version.clone();
        self.minecraft_version_requirement = normalized.minecraft_version_requirement.clone();
        self.loader = normalized.loader.clone();
        self.feature_tags = normalized.feature_tags.clone();
        self.notes = normalized.notes.clone();
        self.revision += 1;
        self.history.push(BuildRestrictionChange {
            revision: self.revision,
            source,
            patch: normalized,
            summary: summary.into(),
        });

        Ok(UpdateBuildRestrictionsOutput {
            missing_fields: missing_restriction_fields(self),
            restrictions: self.clone(),
            warnings,
        })
    }

    /// Project the current restrictions into an update output *without* applying
    /// a patch, deriving `missing_fields` exactly as [`Self::try_apply`] does.
    /// Gates that must surface the existing restrictions plus a contextual
    /// warning (customization/execution blocks) use this so they never re-derive
    /// the output shape by hand.
    pub(super) fn as_update_output(&self, warnings: Vec<String>) -> UpdateBuildRestrictionsOutput {
        UpdateBuildRestrictionsOutput {
            missing_fields: missing_restriction_fields(self),
            restrictions: self.clone(),
            warnings,
        }
    }
}

/// Which hard-requirement fields are still unset. Kept next to [`BuildRestrictions::try_apply`]
/// since both the applied output and the projected output derive from it.
fn missing_restriction_fields(restrictions: &BuildRestrictions) -> Vec<String> {
    let mut missing = Vec::new();
    if restrictions.minecraft_version.is_none() {
        missing.push("minecraft_version".to_string());
    }
    if restrictions.loader.is_none() {
        missing.push("loader".to_string());
    }
    missing
}

/// A permissive "looks like a Minecraft release" check (`1.x[.y[.z]]`).
pub(super) fn is_minecraft_version(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() >= 2
        && parts.len() <= 4
        && parts.first() == Some(&"1")
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Whitelist a loader name to its canonical lowercase form, or `None` if
/// unsupported.
pub(super) fn normalize_loader(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fabric" => Some("fabric".to_string()),
        "forge" => Some("forge".to_string()),
        "neoforge" | "neo forge" => Some("neoforge".to_string()),
        "quilt" => Some("quilt".to_string()),
        _ => None,
    }
}

/// Trim + lowercase feature tags, cap at eight, then dedupe (first occurrence
/// wins). The cap is applied before the dedupe so it matches the pre-refactor
/// authoritative pass exactly.
fn normalize_feature_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    tags.into_iter()
        .map(|tag| tag.trim().to_ascii_lowercase())
        .filter(|tag| !tag.is_empty())
        .take(8)
        .filter(|tag| seen.insert(tag.clone()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildRestrictionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version_requirement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(default)]
    pub feature_tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateBuildRestrictionsInput {
    pub base_revision: u64,
    pub patch: BuildRestrictionPatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateBuildRestrictionsOutput {
    pub restrictions: BuildRestrictions,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BuildRestrictionChange {
    pub revision: u64,
    pub source: BuildRestrictionChangeSource,
    pub patch: BuildRestrictionPatch,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BuildRestrictionChangeSource {
    InitialPrompt,
    UserRevise,
    UiEdit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCompatibility {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minecraft_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loader: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_number: Option<String>,
    #[serde(default)]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_file: Option<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModPlanState {
    pub target: TargetCompatibility,
    #[serde(default)]
    pub base_set: Vec<ResolvedMod>,
    #[serde(default)]
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub additions: Vec<ResolvedMod>,
    #[serde(default)]
    pub removals: Vec<String>,
    #[serde(default)]
    pub blocked: Vec<String>,
    #[serde(default)]
    pub round: u32,
    #[serde(default)]
    pub empty_candidate_rounds: u32,
    #[serde(default)]
    pub pending_queries: Vec<GoalQuery>,
    /// Theme goal ids that a base-pack coverage analysis judged already
    /// satisfied by the selected base pack's own modlist. These goals are
    /// marked [`GoalStatus::Covered`] without adding a new mod, so the planner
    /// never searches for them. Tracked separately from goal status so the
    /// confirmation/validation can distinguish "covered by the base pack" from
    /// "covered by an added mod".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_covered_goals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedMod {
    pub provider: String,
    pub project_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<Value>,
    #[serde(default)]
    pub payload: Value,
    pub provenance: ModProvenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModProvenance {
    BaseSet,
    Baseline,
    Selected,
    Dependency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub label: String,
    pub kind: GoalKind,
    pub status: GoalStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalKind {
    Baseline,
    Theme,
    Dependency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Open,
    Covered,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalQuery {
    pub goal_id: String,
    pub query: String,
}

/// The structured plan approved by a human at the end of
/// `ModpackBuildWorkflow` planning. This is plan metadata: it captures what
/// the user approved plus a deterministic execution recipe. The executor owns
/// any later execution manifest generated from this recipe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovedModpackBuild {
    pub base_pack: serde_json::Value,
    pub target: serde_json::Value,
    #[serde(default)]
    pub extra_mods: Vec<serde_json::Value>,
    #[serde(
        default,
        alias = "mrpack_plan",
        skip_serializing_if = "Option::is_none"
    )]
    pub execution_recipe: Option<serde_json::Value>,
}

/// Execution-owned metadata. It intentionally lives outside
/// [`ApprovedModpackBuild`] so deterministic execution can compile, retry, or
/// block without mutating the human-approved plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExecutionMetadata {
    pub status: AgentExecutionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<ExecutionBlocked>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExecutionStatus {
    NotStarted,
    CompilingManifest,
    Ready,
    Retry,
    Running,
    Blocked,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionBlocked {
    pub phase: AgentPhase,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replan_phase: Option<AgentPhase>,
    #[serde(default)]
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanReplanRequest {
    pub id: String,
    pub reason: String,
    pub from_phase: AgentPhase,
    pub target_phase: AgentPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restriction_patch: Option<BuildRestrictionPatch>,
    #[serde(default)]
    pub invalidates: Vec<PlanArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanArtifact {
    BasePack,
    ExtraMods,
    ApprovedBuild,
    ExecutionMetadata,
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", uuid::Uuid::new_v4())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_snapshot_starts_running_at_intent_phase() {
        let run = AgentRunSnapshot::new("make an aviation colony pack");
        assert_eq!(run.workflow, AgentWorkflowKind::ModpackBuild);
        assert_eq!(run.schema_version, AGENT_SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(run.status, AgentStatus::Running);
        assert_eq!(run.phase, AgentPhase::IntentExtraction);
        assert!(run.pending_approval.is_none());
        assert!(run.restrictions.is_none());
        assert!(run.id.starts_with("agent-run-"));
    }

    #[test]
    fn transition_methods_keep_status_and_pending_approval_in_lockstep() {
        let approval = ApprovalRequest {
            id: "approval-test".to_string(),
            kind: ApprovalKind::ChooseBasePack,
            title: "title".to_string(),
            message: "message".to_string(),
            options: Vec::new(),
            available_decisions: Vec::new(),
            tools: Vec::new(),
            plan: None,
        };
        let plan = ModpackAgentPlan {
            objective: "objective".to_string(),
            summary_markdown: "summary".to_string(),
            risks: Vec::new(),
            planned_actions: Vec::new(),
            migration_notes: Vec::new(),
        };

        let mut run = AgentRunSnapshot::new("make a pack");

        // request_approval pauses at WaitingForUser with the approval + plan held.
        run.request_approval(
            AgentPhase::ChooseBasePackApproval,
            approval.clone(),
            Some(plan.clone()),
        );
        assert_eq!(run.status, AgentStatus::WaitingForUser);
        assert_eq!(run.phase, AgentPhase::ChooseBasePackApproval);
        assert!(run.pending_approval.is_some());
        assert!(run.plan.is_some());

        // enter_phase leaves the gate into Running and clears the pending approval.
        run.enter_phase(AgentPhase::Executing);
        assert_eq!(run.status, AgentStatus::Running);
        assert_eq!(run.phase, AgentPhase::Executing);
        assert!(run.pending_approval.is_none());

        // complete() from a re-entered gate must clear the pending approval.
        run.request_approval(
            AgentPhase::ConfirmCustomizationApproval,
            approval.clone(),
            None,
        );
        assert!(run.pending_approval.is_some());
        run.complete(AgentPhase::Completed);
        assert_eq!(run.status, AgentStatus::Completed);
        assert_eq!(run.phase, AgentPhase::Completed);
        assert!(run.pending_approval.is_none());

        // fail() from a re-entered gate must likewise clear the pending approval.
        run.request_approval(AgentPhase::ConfirmCustomizationApproval, approval, None);
        assert!(run.pending_approval.is_some());
        run.fail(AgentPhase::Failed);
        assert_eq!(run.status, AgentStatus::Failed);
        assert_eq!(run.phase, AgentPhase::Failed);
        assert!(run.pending_approval.is_none());
    }

    #[test]
    fn snapshot_messages_are_soft_capped_and_keep_original_prompt() {
        let mut run = AgentRunSnapshot::new("original prompt");
        run.push_message(AgentMessageKind::User, "original prompt");

        for idx in 0..(MAX_AGENT_MESSAGES + 25) {
            run.push_message(AgentMessageKind::Assistant, format!("revision {idx}"));
        }

        assert_eq!(run.messages.len(), MAX_AGENT_MESSAGES);
        assert_eq!(run.messages[0].kind, AgentMessageKind::User);
        assert_eq!(run.messages[0].text, "original prompt");
        assert_eq!(
            run.messages.last().map(|message| message.text.as_str()),
            Some(format!("revision {}", MAX_AGENT_MESSAGES + 24).as_str())
        );
    }

    #[test]
    fn snapshot_trace_and_replans_are_soft_capped() {
        let mut run = AgentRunSnapshot::new("make a pack");

        for idx in 0..(MAX_AGENT_TRACE_EVENTS + 10) {
            run.push_trace(format!("trace {idx}"));
        }
        for idx in 0..(MAX_AGENT_REPLANS + 10) {
            run.push_replan(PlanReplanRequest {
                id: format!("replan-{idx}"),
                reason: format!("reason {idx}"),
                from_phase: AgentPhase::ChooseBasePackApproval,
                target_phase: AgentPhase::ConfigureRequirementsApproval,
                restriction_patch: None,
                invalidates: vec![PlanArtifact::BasePack],
            });
        }

        assert_eq!(run.trace.len(), MAX_AGENT_TRACE_EVENTS);
        assert_eq!(
            run.trace.first().map(|event| event.event.as_str()),
            Some("trace 10")
        );
        assert_eq!(
            run.trace.last().map(|event| event.event.as_str()),
            Some(format!("trace {}", MAX_AGENT_TRACE_EVENTS + 9).as_str())
        );
        assert_eq!(run.replans.len(), MAX_AGENT_REPLANS);
        assert_eq!(
            run.replans.first().map(|replan| replan.id.as_str()),
            Some("replan-10")
        );
        assert_eq!(
            run.replans.last().map(|replan| replan.id.as_str()),
            Some(format!("replan-{}", MAX_AGENT_REPLANS + 9).as_str())
        );
    }

    /// Table-driven coverage of the single authoritative normalization pass that
    /// `try_apply` now owns. Every case starts from a fresh default (revision 0)
    /// and asserts the final stored fields, plus derived `missing_fields` and
    /// `warnings`, match the pre-refactor `update_build_restrictions` behavior.
    #[test]
    fn try_apply_runs_single_authoritative_normalization_pass() {
        struct Case {
            name: &'static str,
            patch: BuildRestrictionPatch,
            version: Option<&'static str>,
            requirement: Option<&'static str>,
            loader: Option<&'static str>,
            tags: Vec<&'static str>,
            notes: Option<&'static str>,
            missing: Vec<&'static str>,
            warnings: Vec<&'static str>,
        }

        let cases = vec![
            Case {
                name: "valid target; tags trimmed+lowercased+deduped; requirement backfilled",
                patch: BuildRestrictionPatch {
                    minecraft_version: Some("1.20.1".to_string()),
                    minecraft_version_requirement: None,
                    loader: Some("Fabric".to_string()),
                    feature_tags: vec![
                        " Perf ".to_string(),
                        "perf".to_string(),
                        "QoL".to_string(),
                    ],
                    notes: Some("  keep this  ".to_string()),
                },
                version: Some("1.20.1"),
                requirement: Some("1.20.1"),
                loader: Some("fabric"),
                tags: vec!["perf", "qol"],
                notes: Some("keep this"),
                missing: vec![],
                warnings: vec![],
            },
            Case {
                name: "invalid version dropped WITH a warning (authoritative), not silently",
                patch: BuildRestrictionPatch {
                    minecraft_version: Some("99.99".to_string()),
                    minecraft_version_requirement: None,
                    loader: Some("forge".to_string()),
                    feature_tags: vec![],
                    notes: None,
                },
                version: None,
                requirement: None,
                loader: Some("forge"),
                tags: vec![],
                notes: None,
                missing: vec!["minecraft_version"],
                warnings: vec!["ignored invalid minecraft_version: 99.99"],
            },
            Case {
                name: "unsupported loader dropped with a warning; raw requirement preserved",
                patch: BuildRestrictionPatch {
                    minecraft_version: None,
                    minecraft_version_requirement: Some(" 1.20.x ".to_string()),
                    loader: Some("modloader".to_string()),
                    feature_tags: vec!["adventure".to_string()],
                    notes: None,
                },
                version: None,
                requirement: Some("1.20.x"),
                loader: None,
                tags: vec!["adventure"],
                notes: None,
                missing: vec!["minecraft_version", "loader"],
                warnings: vec!["ignored unsupported loader"],
            },
            Case {
                name: "tags capped at 8 BEFORE dedupe, then case-folded dedupe collapses",
                patch: BuildRestrictionPatch {
                    minecraft_version: Some("1.19.2".to_string()),
                    minecraft_version_requirement: Some("1.19.2".to_string()),
                    loader: Some("NeoForge".to_string()),
                    feature_tags: vec![
                        "A".to_string(),
                        "a".to_string(),
                        "B".to_string(),
                        "b".to_string(),
                        "C".to_string(),
                        "c".to_string(),
                        "D".to_string(),
                        "d".to_string(),
                        "E".to_string(),
                        "e".to_string(),
                    ],
                    notes: None,
                },
                version: Some("1.19.2"),
                requirement: Some("1.19.2"),
                loader: Some("neoforge"),
                tags: vec!["a", "b", "c", "d"],
                notes: None,
                missing: vec![],
                warnings: vec![],
            },
            Case {
                name: "empty patch leaves both hard fields missing",
                patch: BuildRestrictionPatch {
                    minecraft_version: None,
                    minecraft_version_requirement: None,
                    loader: None,
                    feature_tags: vec![],
                    notes: None,
                },
                version: None,
                requirement: None,
                loader: None,
                tags: vec![],
                notes: None,
                missing: vec!["minecraft_version", "loader"],
                warnings: vec![],
            },
        ];

        for case in cases {
            let mut restrictions = BuildRestrictions::default();
            let output = restrictions
                .try_apply(
                    0,
                    case.patch,
                    BuildRestrictionChangeSource::InitialPrompt,
                    "test",
                )
                .unwrap_or_else(|err| panic!("{}: try_apply should succeed: {err}", case.name));

            assert_eq!(
                output.restrictions.minecraft_version.as_deref(),
                case.version,
                "{}: minecraft_version",
                case.name
            );
            assert_eq!(
                output.restrictions.minecraft_version_requirement.as_deref(),
                case.requirement,
                "{}: minecraft_version_requirement",
                case.name
            );
            assert_eq!(
                output.restrictions.loader.as_deref(),
                case.loader,
                "{}: loader",
                case.name
            );
            let tags: Vec<&str> = output
                .restrictions
                .feature_tags
                .iter()
                .map(String::as_str)
                .collect();
            assert_eq!(tags, case.tags, "{}: feature_tags", case.name);
            assert_eq!(
                output.restrictions.notes.as_deref(),
                case.notes,
                "{}: notes",
                case.name
            );
            let missing: Vec<&str> = output.missing_fields.iter().map(String::as_str).collect();
            assert_eq!(missing, case.missing, "{}: missing_fields", case.name);
            let warnings: Vec<&str> = output.warnings.iter().map(String::as_str).collect();
            assert_eq!(warnings, case.warnings, "{}: warnings", case.name);

            // The revision always advances by one and the returned view mirrors
            // the mutated receiver exactly.
            assert_eq!(output.restrictions.revision, 1, "{}: revision", case.name);
            assert_eq!(output.restrictions, restrictions, "{}: mirrors self", case.name);
        }
    }

    #[test]
    fn try_apply_bumps_revision_and_appends_normalized_history() {
        let mut restrictions = BuildRestrictions {
            revision: 4,
            ..Default::default()
        };
        let output = restrictions
            .try_apply(
                4,
                BuildRestrictionPatch {
                    minecraft_version: Some("1.20.1".to_string()),
                    minecraft_version_requirement: None,
                    loader: Some("Fabric".to_string()),
                    feature_tags: vec![" Combat ".to_string(), "combat".to_string()],
                    notes: None,
                },
                BuildRestrictionChangeSource::UserRevise,
                "revise to fabric 1.20.1",
            )
            .expect("apply should succeed on a matching base revision");

        assert_eq!(restrictions.revision, 5);
        assert_eq!(output.restrictions.revision, 5);
        assert_eq!(restrictions.history.len(), 1);
        let change = &restrictions.history[0];
        assert_eq!(change.revision, 5);
        assert_eq!(change.source, BuildRestrictionChangeSource::UserRevise);
        assert_eq!(change.summary, "revise to fabric 1.20.1");
        // History stores the NORMALIZED patch, not the raw model output.
        assert_eq!(change.patch.loader.as_deref(), Some("fabric"));
        assert_eq!(change.patch.feature_tags, vec!["combat".to_string()]);
        assert_eq!(
            change.patch.minecraft_version_requirement.as_deref(),
            Some("1.20.1")
        );
    }

    #[test]
    fn try_apply_rejects_revision_mismatch_without_mutating() {
        let mut restrictions = BuildRestrictions {
            revision: 3,
            minecraft_version: Some("1.20.1".to_string()),
            minecraft_version_requirement: Some("1.20.1".to_string()),
            loader: Some("fabric".to_string()),
            ..Default::default()
        };
        let before = restrictions.clone();
        let err = restrictions
            .try_apply(
                2,
                BuildRestrictionPatch {
                    minecraft_version: Some("1.19.2".to_string()),
                    minecraft_version_requirement: None,
                    loader: Some("forge".to_string()),
                    feature_tags: vec![],
                    notes: None,
                },
                BuildRestrictionChangeSource::UserRevise,
                "stale write",
            )
            .expect_err("a stale base_revision must be rejected");

        assert!(
            err.to_string().contains("revision mismatch"),
            "unexpected error: {err}"
        );
        // The optimistic-concurrency guard leaves the receiver untouched.
        assert_eq!(restrictions, before);
    }
}
