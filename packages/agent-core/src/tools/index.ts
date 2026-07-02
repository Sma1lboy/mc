// Tool registry — one self-contained file per tool (native AI SDK `tool()` with
// its own `execute` inlined). Assembled here into the per-turn `ToolSet`. Adding
// a tool = add a file + one line in `buildTools`.

import type { ToolSet } from "ai";
import type { z } from "zod";

import type { AgentToolContext, AgentToolName, ToolExecutor } from "../types";
import { searchBaseModpacks } from "./search-base-modpacks";
import { inspectBaseModpack } from "./inspect-base-modpack";
import { searchMods } from "./search-mods";
import { modGetDetail } from "./mod-get-detail";
import { resolveMods } from "./resolve-mods";
import { buildModpack } from "./build-modpack";
import { wikiSearch } from "./wiki-search";
import { wikiOpen } from "./wiki-open";
import { askUserQuestion } from "./ask-user-question";

export { ASK_USER_TOOL } from "./ask-user-question";

export const BUILD_TOOL_NAMES = [
  "search_base_modpacks",
  "inspect_base_modpack",
  "search_mods",
  "mod_get_detail",
  "resolve_mods",
  "build_modpack",
  "ask_user_question",
] as const satisfies readonly AgentToolName[];

export const WIKI_TOOL_NAMES = ["wiki_search", "wiki_open"] as const satisfies readonly AgentToolName[];

export const ALL_TOOL_NAMES = [
  ...BUILD_TOOL_NAMES,
  ...WIKI_TOOL_NAMES,
] as const satisfies readonly AgentToolName[];

/**
 * Build the AI SDK `ToolSet` for one turn. Host tools take the injected `exec`
 * backend; the client tool (`ask_user_question`) ignores it. Listed explicitly
 * (not a loop) so each tool keeps its own concrete input type for SDK inference.
 */
export function buildTools(
  exec: ToolExecutor,
  context?: AgentToolContext,
  toolNames: readonly AgentToolName[] = BUILD_TOOL_NAMES,
): ToolSet {
  const registry: Record<AgentToolName, ToolSet[string]> = {
    search_base_modpacks: searchBaseModpacks(exec),
    inspect_base_modpack: inspectBaseModpack(exec),
    search_mods: searchMods(exec),
    mod_get_detail: modGetDetail(exec),
    resolve_mods: resolveMods(exec),
    build_modpack: buildModpack(exec),
    wiki_search: wikiSearch(exec, context),
    wiki_open: wikiOpen(exec, context),
    ask_user_question: askUserQuestion(),
  };
  return Object.fromEntries(toolNames.map((name) => [name, registry[name]])) as ToolSet;
}

/**
 * Each tool's zod input schema, keyed by name — for validating a raw tool-call
 * payload (and unit tests). Derived from the built tools so every schema stays
 * single-sourced inside its own tool file; building never invokes `execute`, so
 * the empty executor here is only ever used to read `inputSchema`.
 */
export const toolSchemas: Record<string, z.ZodType> = Object.fromEntries(
  Object.entries(buildTools({}, undefined, ALL_TOOL_NAMES)).map(([name, t]) => [
    name,
    t.inputSchema as z.ZodType,
  ]),
);
