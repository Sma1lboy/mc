//! A lean, streaming, tool-use chat agent for the kobeMC modpack workflow.
//!
//! This is the *agentic* counterpart to the fixed-pipeline state machine in
//! [`crate::agent::workflow`]. Instead of a rigid phase/approval reducer, the flow
//! lives in a system prompt ([`CHAT_AGENT_SYSTEM_PROMPT`]) and a handful of
//! deterministic tools; a streaming tool-use loop ([`run_chat_turn`]) lets the
//! model chat, orchestrate those tools, and ask the user. **Safety comes from the
//! tools returning only real provider/resolver data** (they never let the model
//! fabricate ids/urls/hashes) and from `build_modpack` being gated behind explicit
//! user confirmation — not from a state machine.
//!
//! The two coexist: this module is purely additive and reuses the same
//! deterministic `mc-core` primitives (provider search, dependency resolution,
//! the base-modlist parser, the `.mrpack` executor).
//!
//! Streaming approach (STEP 0 finding): Rig 0.39 provides a built-in streaming
//! multi-turn tool-use loop, so we use it directly rather than hand-rolling SSE
//! parsing — see [`run`].

mod prompt;
mod run;
mod store;
mod tools;

#[cfg(test)]
mod tests;

pub use prompt::CHAT_AGENT_SYSTEM_PROMPT;
pub use run::{run_chat_turn, ChatEventSink, ChatTranscript, ChatTurnOutcome, CollectingSink};
pub use store::{delete_transcript, load_transcript, save_transcript};
pub use tools::{
    BuildBasePack, BuildModRef, BuildModpackArgs, BuildModpackOutput, BuildModpackTool,
    BuildTarget, ChatToolError, ChatToolsCtx, InspectBaseModpackArgs, InspectBaseModpackOutput,
    InspectBaseModpackTool, ModGetDetailArgs, ModGetDetailOutput, ModGetDetailTool, ModHit,
    ResolveModsArgs, ResolveModsOutput, ResolveModsTool,
    SearchBaseModpacksArgs, SearchBaseModpacksOutput, SearchBaseModpacksTool, SearchModsArgs,
    SearchModsOutput, SearchModsTool,
};

/// The transcript message type, re-exported so callers can persist/inspect
/// history without depending on `rig-core` paths directly.
pub use rig_core::completion::Message as ChatMessage;
