//! Serializable state for main-agent runs and subworkflow sessions.
//!
//! The launcher daemon remains the source of truth for game operations. The
//! agent can return approval gates, but it does not own installation or writes.

use std::time::{SystemTime, UNIX_EPOCH};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentInterruptKind {
    UserApproval,
    UserInput,
    UserClarification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentInputKind {
    SelectMinecraftVersion,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInputOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInterrupt {
    pub id: String,
    pub kind: AgentInterruptKind,
    pub title: String,
    pub message: String,
    pub resume_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_kind: Option<AgentInputKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<AgentInputOption>,
    #[serde(default)]
    pub allow_freeform: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_kind: Option<ApprovalKind>,
}

impl AgentInterrupt {
    pub fn user_input(
        input_kind: AgentInputKind,
        title: impl Into<String>,
        message: impl Into<String>,
        options: Vec<AgentInputOption>,
        allow_freeform: bool,
        value: Option<serde_json::Value>,
    ) -> Self {
        let id = new_id("interrupt");
        Self {
            id: id.clone(),
            kind: AgentInterruptKind::UserInput,
            title: title.into(),
            message: message.into(),
            resume_token: id,
            input_kind: Some(input_kind),
            value,
            options,
            allow_freeform,
            approval_id: None,
            approval_kind: None,
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_interrupt: Option<AgentInterrupt>,
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
    /// Opaque tool-loop memory for the modpack agent. This is not workflow
    /// control state; it stores factual tool outputs that the next model turn can
    /// inspect after resume.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub agent_memory: serde_json::Value,
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
            pending_interrupt: None,
            tools: Vec::new(),
            plan: None,
            restrictions: None,
            mod_plan: None,
            approved_build: None,
            execution: None,
            agent_memory: serde_json::Value::Null,
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
            stream_kind: Some(AgentStreamEventKind::Milestone),
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

    pub fn push_tool_call_started(
        &mut self,
        stage: AgentPhase,
        iteration: u32,
        tool: impl Into<String>,
        input: Value,
    ) {
        self.trace.push(AgentTraceEvent {
            at_ms: now_ms(),
            stream_kind: Some(AgentStreamEventKind::ToolCallStarted),
            event: "tool call started".to_string(),
            stage: Some(stage),
            iteration: Some(iteration),
            tool: Some(tool.into()),
            input: Some(input),
            output: None,
            duration_ms: None,
            status: Some("started".to_string()),
        });
        self.trim_trace();
    }

    pub fn push_stream_event(
        &mut self,
        stream_kind: AgentStreamEventKind,
        event: impl Into<String>,
        stage: Option<AgentPhase>,
        output: Value,
    ) {
        self.trace.push(AgentTraceEvent {
            at_ms: now_ms(),
            stream_kind: Some(stream_kind),
            event: event.into(),
            stage,
            iteration: None,
            tool: None,
            input: None,
            output: Some(output),
            duration_ms: None,
            status: None,
        });
        self.trim_trace();
    }

    pub fn push_tool_trace(&mut self, trace: AgentToolTrace) {
        self.trace.push(AgentTraceEvent {
            at_ms: now_ms(),
            stream_kind: Some(AgentStreamEventKind::ToolCallResult),
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
        let interrupt = approval_interrupt(&approval);
        let interrupt_id = interrupt.id.clone();
        let approval_id = approval.id.clone();
        let approval_kind =
            serde_json::to_value(&approval.kind).unwrap_or_else(|_| serde_json::json!("unknown"));
        self.status = AgentStatus::WaitingForUser;
        self.phase = phase;
        self.pending_approval = Some(approval);
        self.pending_interrupt = Some(interrupt);
        self.plan = plan;
        self.trace.push(AgentTraceEvent {
            at_ms: now_ms(),
            stream_kind: Some(AgentStreamEventKind::ApprovalRequired),
            event: "approval required".to_string(),
            stage: Some(self.phase.clone()),
            iteration: None,
            tool: None,
            input: None,
            output: Some(serde_json::json!({
                "interrupt_id": interrupt_id,
                "interrupt_kind": "user_approval",
                "approval_id": approval_id,
                "approval_kind": approval_kind,
            })),
            duration_ms: None,
            status: Some("waiting_for_user".to_string()),
        });
        self.trim_trace();
    }

    pub fn request_input(&mut self, phase: AgentPhase, interrupt: AgentInterrupt) {
        let interrupt_id = interrupt.id.clone();
        let input_kind =
            serde_json::to_value(&interrupt.input_kind).unwrap_or_else(|_| serde_json::Value::Null);
        self.status = AgentStatus::WaitingForUser;
        self.phase = phase;
        self.pending_approval = None;
        self.pending_interrupt = Some(interrupt);
        self.trace.push(AgentTraceEvent {
            at_ms: now_ms(),
            stream_kind: Some(AgentStreamEventKind::InputRequired),
            event: "input required".to_string(),
            stage: Some(self.phase.clone()),
            iteration: None,
            tool: None,
            input: None,
            output: Some(serde_json::json!({
                "interrupt_id": interrupt_id,
                "interrupt_kind": "user_input",
                "input_kind": input_kind,
            })),
            duration_ms: None,
            status: Some("waiting_for_user".to_string()),
        });
        self.trim_trace();
    }

    pub fn clear_user_interrupt(&mut self) {
        self.status = AgentStatus::Running;
        self.pending_approval = None;
        self.pending_interrupt = None;
    }

    /// Leave any gate into a running `phase`, clearing the pending approval so the
    /// `Running` ⇒ no-pending-approval half of the invariant holds by construction.
    pub fn enter_phase(&mut self, phase: AgentPhase) {
        self.status = AgentStatus::Running;
        self.phase = phase;
        self.pending_approval = None;
        self.pending_interrupt = None;
    }

    /// Terminate the run as completed at `phase`, clearing any pending approval.
    pub fn complete(&mut self, phase: AgentPhase) {
        self.status = AgentStatus::Completed;
        self.phase = phase;
        self.pending_approval = None;
        self.pending_interrupt = None;
    }

    /// Terminate the run as failed at `phase`, clearing any pending approval.
    pub fn fail(&mut self, phase: AgentPhase) {
        self.status = AgentStatus::Failed;
        self.phase = phase;
        self.pending_approval = None;
        self.pending_interrupt = None;
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

fn approval_interrupt(approval: &ApprovalRequest) -> AgentInterrupt {
    AgentInterrupt {
        id: new_id("interrupt"),
        kind: AgentInterruptKind::UserApproval,
        title: approval.title.clone(),
        message: approval.message.clone(),
        resume_token: approval.id.clone(),
        input_kind: None,
        value: None,
        options: Vec::new(),
        allow_freeform: false,
        approval_id: Some(approval.id.clone()),
        approval_kind: Some(approval.kind.clone()),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStreamEventKind {
    MessageDelta,
    ToolCallStarted,
    ToolCallResult,
    Milestone,
    InputRequired,
    ClarificationNeeded,
    ApprovalRequired,
    Final,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    pub kind: AgentMessageKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTraceEvent {
    pub at_ms: u128,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream_kind: Option<AgentStreamEventKind>,
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

mod restrictions;

pub use restrictions::{
    BuildRestrictionChange, BuildRestrictionChangeSource, BuildRestrictionPatch, BuildRestrictions,
    BuildRestrictionsLlmView, UpdateBuildRestrictionsInput, UpdateBuildRestrictionsOutput,
};
pub(in crate::agent) use restrictions::{is_minecraft_version, normalize_loader};

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
/// modpack-build agent planning. This is plan metadata: it captures what
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
mod tests;
