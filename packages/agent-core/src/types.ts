// Host-agnostic type surface for the modpack-agent "brain".
//
// This directory (agent/core/) MUST NOT import react / @tauri-apps / any project
// UI code — only `ai`, `zod`, and std TS. Everything host-specific (Tauri invoke,
// an HTTP route, the daemon) lives in an adapter that injects a `ToolExecutor`.

import { z } from "zod";

/**
 * Option schema for an `ask_user_question` choice — the single source for the
 * tool's option schema and the option type. Derive the type with `z.infer`;
 * don't re-declare it.
 */
export const askUserOptionSchema = z.object({
  label: z.string().describe("The visible choice text, in the user's language."),
  id: z.string().optional().describe("Optional stable id; defaults to the label."),
  description: z.string().optional().describe("Optional one-line detail shown under the label."),
});
export type AskUserOption = z.infer<typeof askUserOptionSchema>;

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

/** Named agent profile selected by the host entrypoint. */
export type AgentProfile = "build" | "wiki";

/** Tool names that can be injected into an agent profile. */
export type AgentToolName =
  | "search_base_modpacks"
  | "inspect_base_modpack"
  | "search_mods"
  | "mod_get_detail"
  | "resolve_mods"
  | "build_modpack"
  | "wiki_search"
  | "wiki_open"
  | "ask_user_question";

/** Prompt + tool slice injected for a concrete agent profile. */
export interface AgentInjection {
  systemPrompt: string;
  toolNames: readonly AgentToolName[];
}

/** Host-owned wiki scope and source selection for current-modpack wiki tools. */
export interface WikiToolContext {
  modpackId: string;
  instanceId?: string;
  sourcePaths: string[];
}

/** Optional host context injected into deterministic tools. */
export interface AgentToolContext {
  profile?: AgentProfile;
  wiki?: WikiToolContext;
}
