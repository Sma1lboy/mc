//! Serializable state for main-agent runs and subworkflow sessions.
//!
//! The launcher daemon remains the source of truth for game operations. The
//! agent can return approval gates, but it does not own installation or writes.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const AGENT_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<AgentIntent>,
    pub status: AgentStatus,
    pub phase: AgentPhase,
    pub user_prompt: String,
    #[serde(default)]
    pub messages: Vec<AgentMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_approval: Option<ApprovalRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ModpackAgentPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restrictions: Option<BuildRestrictions>,
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
            intent: None,
            status: AgentStatus::Running,
            phase: AgentPhase::IntentExtraction,
            user_prompt: user_prompt.into(),
            messages: Vec::new(),
            pending_approval: None,
            plan: None,
            restrictions: None,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateBuildRestrictionsInput {
    pub base_revision: u64,
    pub patch: BuildRestrictionPatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateBuildRestrictionsOutput {
    pub restrictions: BuildRestrictions,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildRestrictionChange {
    pub revision: u64,
    pub source: BuildRestrictionChangeSource,
    pub patch: BuildRestrictionPatch,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildRestrictionChangeSource {
    InitialPrompt,
    UserRevise,
    UiEdit,
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
}
