// Tool registry — one self-contained schema per tool. Every tool is a native AI
// SDK client-side tool: agent-core emits tool calls, and the launcher client
// supplies outputs through Rust IPC before resuming the conversation.

import type { ToolSet } from "ai";
import type { z } from "zod";

import { searchBaseModpacks } from "./search-base-modpacks";
import { inspectBaseModpack } from "./inspect-base-modpack";
import { searchMods } from "./search-mods";
import { modGetDetail } from "./mod-get-detail";
import { resolveMods } from "./resolve-mods";
import { buildModpack } from "./build-modpack";
import { showModpack } from "./show-modpack";
import { listInstances } from "./list-instances";
import { askUserQuestion } from "./ask-user-question";

export { ASK_USER_TOOL } from "./ask-user-question";
export { SHOW_MODPACK_TOOL } from "./show-modpack";

/**
 * Build the AI SDK `ToolSet` for one turn. Listed explicitly so each tool keeps
 * its concrete input type for SDK inference.
 */
export function buildTools(): ToolSet {
  return {
    search_base_modpacks: searchBaseModpacks(),
    inspect_base_modpack: inspectBaseModpack(),
    search_mods: searchMods(),
    mod_get_detail: modGetDetail(),
    resolve_mods: resolveMods(),
    build_modpack: buildModpack(),
    show_modpack: showModpack(),
    list_instances: listInstances(),
    ask_user_question: askUserQuestion(),
  };
}

/**
 * Each tool's zod input schema, keyed by name — for validating a raw tool-call
 * payload (and unit tests). Derived from the built tools so every schema stays
 * single-sourced inside its own tool file; building never invokes `execute`.
 */
export const toolSchemas: Record<string, z.ZodType> = Object.fromEntries(
  Object.entries(buildTools()).map(([name, t]) => [name, t.inputSchema as z.ZodType]),
);
