// Tool registry — one self-contained file per tool (native AI SDK `tool()` with
// its own `execute` inlined). Assembled here into the per-turn `ToolSet`. Adding
// a tool = add a file + one line in `buildTools`.

import type { ToolSet } from "ai";
import type { z } from "zod";

import type { ToolExecutor } from "../types";
import { searchBaseModpacks } from "./search-base-modpacks";
import { inspectBaseModpack } from "./inspect-base-modpack";
import { searchMods } from "./search-mods";
import { modGetDetail } from "./mod-get-detail";
import { resolveMods } from "./resolve-mods";
import { buildModpack } from "./build-modpack";
import { askUserQuestion } from "./ask-user-question";

export { ASK_USER_TOOL } from "./ask-user-question";

/**
 * Build the AI SDK `ToolSet` for one turn. Host tools take the injected `exec`
 * backend; the client tool (`ask_user_question`) ignores it. Listed explicitly
 * (not a loop) so each tool keeps its own concrete input type for SDK inference.
 */
export function buildTools(exec: ToolExecutor): ToolSet {
  return {
    search_base_modpacks: searchBaseModpacks(exec),
    inspect_base_modpack: inspectBaseModpack(exec),
    search_mods: searchMods(exec),
    mod_get_detail: modGetDetail(exec),
    resolve_mods: resolveMods(exec),
    build_modpack: buildModpack(exec),
    ask_user_question: askUserQuestion(),
  };
}

/**
 * Each tool's zod input schema, keyed by name — for validating a raw tool-call
 * payload (and unit tests). Derived from the built tools so every schema stays
 * single-sourced inside its own tool file; building never invokes `execute`, so
 * the empty executor here is only ever used to read `inputSchema`.
 */
export const toolSchemas: Record<string, z.ZodType> = Object.fromEntries(
  Object.entries(buildTools({})).map(([name, t]) => [name, t.inputSchema as z.ZodType]),
);
