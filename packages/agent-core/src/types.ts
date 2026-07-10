// Type surface for the modpack-agent brain.
//
// This package must not import React / @tauri-apps / project UI code. Tool
// definitions live here as schema/protocol; the launcher client provides output
// for every tool call through its Rust IPC boundary.

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

/** Client-provided tool handler used by hosts that bridge a local runtime. */
export type ClientToolHandler = (args: unknown) => Promise<unknown>;
export type ClientToolHandlers = Partial<Record<string, ClientToolHandler>>;

/** Entry-specific agent surface. Each mode gets its own prompt and tool list. */
export type AgentMode = "build" | "instance";

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
