//! Main-agent primitives and callable tool runners.
//!
//! This module is intentionally Rust-native and small. It owns the stable
//! domain schema: run state and approval gates. The MVP has one routed
//! modpack-build tool runner; new capabilities should be added as tools under
//! the main agent facade, not folded into a staged Rust workflow.

pub mod llm;
pub mod session;
pub mod state;
pub mod workflow;

pub use llm::{AgentLlmClient, AgentLlmConfig};
pub use session::{AgentSessionStore, AgentSessionSummary};
pub use state::{
    AgentEntry, AgentExecutionMetadata, AgentExecutionStatus, AgentInputKind, AgentInputOption,
    AgentIntent, AgentIntentKind, AgentInterrupt, AgentInterruptKind, AgentLaunchContext,
    AgentMessage, AgentMessageKind, AgentPhase, AgentRunSnapshot, AgentStatus,
    AgentStreamEventKind, AgentToolSpec, AgentToolTrace, AgentWorkflowId, AgentWorkflowKind,
    ApprovalDecisionSpec, ApprovalKind, ApprovalOption, ApprovalRequest, ApprovedModpackBuild,
    BuildRestrictionChange, BuildRestrictionChangeSource, BuildRestrictionPatch, BuildRestrictions,
    ExecutionBlocked, ModpackAgentPlan, PlanArtifact, PlanReplanRequest, PlannedAction,
    UpdateBuildRestrictionsInput, UpdateBuildRestrictionsOutput, UserDecision, UserDecisionKind,
};
pub use workflow::{
    EXPORT_MRPACK_ARTIFACT_TOOL, MainAgentRuntime, apply_modpack_build_user_decision,
    apply_modpack_build_user_input, compile_mrpack_execution_metadata,
    continue_after_execution_manifest_result, execute_mrpack_build_to_path,
};
