//! Main-agent primitives and callable subworkflows.
//!
//! This module is intentionally Rust-native and small. It owns the stable
//! domain schema: run state and approval gates. The MVP has one routed
//! subworkflow, `ModpackBuildWorkflow`; new capabilities should be added as
//! tools/subworkflows under the main agent facade, not folded into a single
//! monolithic loop.

pub mod llm;
pub mod session;
pub mod state;
pub mod workflow;

pub use llm::{AgentLlmClient, AgentLlmConfig};
pub use session::{AgentSessionStore, AgentSessionSummary};
pub use state::{
    AgentExecutionMetadata, AgentExecutionStatus, AgentIntent, AgentIntentKind, AgentMessage,
    AgentMessageKind, AgentPhase, AgentRunSnapshot, AgentStatus, AgentToolSpec, AgentToolTrace,
    AgentWorkflowKind, ApprovalDecisionSpec, ApprovalKind, ApprovalOption, ApprovalRequest,
    ApprovedModpackBuild, BuildRestrictionChange, BuildRestrictionChangeSource,
    BuildRestrictionPatch, BuildRestrictions, ExecutionBlocked, ModpackAgentPlan, PlanArtifact,
    PlanReplanRequest, PlannedAction, UpdateBuildRestrictionsInput, UpdateBuildRestrictionsOutput,
    UserDecision, UserDecisionKind,
};
pub use workflow::{
    compile_mrpack_execution_metadata, continue_after_execution_manifest_result,
    continue_modpack_build_without_model, execute_mrpack_build_to_path, MainAgentRuntime,
    ModpackAgentRuntime, ModpackBuildWorkflow,
};
