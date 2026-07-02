//! Deterministic, real-data tools for the kobeMC modpack agent.
//!
//! Once the home of a rust streaming chat *brain* (rig tool-use loop) too; that
//! brain was retired in favour of the TS brain (`@kobemc/agent-core`, runs in the
//! webview). What remains here is the **deterministic tool layer**: `tool_*` fns +
//! their arg/output types, exposed to the TS brain via the `agent_tool_*` Tauri
//! commands. **Safety lives here** — the tools return only real provider/resolver
//! data (never let the model fabricate ids/urls/hashes) and `build_modpack` stays
//! Rust-side. Reuses the same `mc-core` primitives (provider search, dependency
//! resolution, base-modlist parser, `.mrpack` executor).

mod tools;

#[cfg(test)]
mod tests;

pub use tools::{
    tool_build_modpack, tool_inspect_base_modpack, tool_mod_get_detail, tool_resolve_mods,
    tool_search_base_modpacks, tool_search_mods, BuildBasePack, BuildModRef, BuildModpackArgs,
    BuildModpackOutput, BuildModpackTool, BuildTarget, ChatToolError, ChatToolsCtx,
    InspectBaseModpackArgs, InspectBaseModpackOutput, InspectBaseModpackTool, ModGetDetailArgs,
    ModGetDetailOutput, ModGetDetailTool, ModHit, ResolveModsArgs, ResolveModsOutput,
    ResolveModsTool, SearchBaseModpacksArgs, SearchBaseModpacksOutput, SearchBaseModpacksTool,
    SearchModsArgs, SearchModsOutput, SearchModsTool,
};
