// Host-agnostic type surface for the modpack-agent "brain".
//
// This directory (agent/core/) MUST NOT import react / @tauri-apps / any project
// UI code — only `ai`, `zod`, and std TS. Everything host-specific (Tauri invoke,
// an HTTP route, the daemon) lives in an adapter that injects a `ToolExecutor`.

import type { ModelMessage } from "ai";

/**
 * Streamed turn events, wire-identical to Rust `mc_types::AgentStreamEvent`
 * (an internally-tagged snake_case union). Keeping the SAME tags lets ONE
 * reducer in the UI serve both the Rust brain (events arrive over a Tauri
 * Channel) and this TS brain (events call the reducer directly).
 *
 * `tool_call.args` is arbitrary JSON, typed `unknown` here so core stays free of
 * the project's `JsonValue`; the desktop seam casts it when handing off.
 */
export type AgentStreamEvent =
  | { type: "text_delta"; delta: string }
  | { type: "reasoning"; delta: string }
  | { type: "tool_call"; name: string; args: unknown }
  | { type: "tool_result"; name: string; summary: string }
  | { type: "done" }
  | { type: "error"; message: string };

/**
 * The transcript currency: the AI SDK's `ModelMessage` (a.k.a. CoreMessage).
 * Using it directly means assistant tool-call turns and tool-result turns
 * round-trip losslessly between turns (mirrors the Rust `ChatTranscript`).
 */
export type ChatMessage = ModelMessage;

/**
 * Host-injected tool backend: a map of tool name → async executor. The core
 * never talks to Tauri/HTTP; the host binds each of the six tool names to a real
 * call (desktop → `invoke`, mc-server → its own resolver, …). Args are the JSON
 * the model produced (validated again on the Rust side); the return is the tool's
 * output JSON, echoed into a `tool_result` summary.
 */
export type ToolExecutor = Record<string, (args: unknown) => Promise<unknown>>;

/**
 * LLM endpoint config. Mirrors mc-core `AgentLlmConfig` (`api_key`/`model`/
 * `base_url`), camelCased for TS. Any OpenAI-compatible base URL works, so the
 * same core runs against OpenRouter today and a self-hosted endpoint later.
 */
export interface AgentLlmSettings {
  apiKey: string;
  model: string;
  baseUrl: string;
}
