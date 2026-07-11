//! Server-side tool executor for the modpack agent.
//!
//! The agent *brain* is TypeScript (`@kobemc/agent-core`, runs in the webview /
//! any TS host); this module is only its deterministic tool layer plus the LLM
//! config loader. `tools` exposes the `tool_*` fns wired to the `agent_tool_*`
//! Tauri commands; `build` is the trusted `.mrpack` executor behind them.

mod build;
pub mod compatibility;
pub mod llm;
pub mod runtime;
pub mod tools;

pub use llm::AgentLlmConfig;
pub use runtime::{detect_local_runtime, LocalRuntimeStatus};
pub use tools::{ChatToolError, ChatToolsCtx};
