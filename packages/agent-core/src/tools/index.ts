// Tool registry — one self-contained schema per tool. Every tool is a native AI
// SDK client-side tool: agent-core emits tool calls, and the launcher client
// supplies outputs through Rust IPC before resuming the conversation.

import type { ToolSet } from "ai";
import type { z } from "zod";

import { normalizeAgentMode, type AgentModeInput } from "../types";
import { askUserQuestion } from "./ask-user-question";
import { buildModpack } from "./build-modpack";
import { finishDeepDiagnosis, runDiagnosticTrial, startDeepDiagnosis } from "./deep-diagnosis";
import { diagnoseInstance } from "./diagnose-instance";
import { inspectBaseModpack } from "./inspect-base-modpack";
import { listInstances } from "./list-instances";
import { modGetDetail } from "./mod-get-detail";
import { resolveMods } from "./resolve-mods";
import { searchBaseModpacks } from "./search-base-modpacks";
import { searchMods } from "./search-mods";
import { showInstanceChanges } from "./show-instance-changes";
import { showModpack } from "./show-modpack";
import { validateModpackPlan } from "./validate-modpack-plan";
import { wikiOpen } from "./wiki-open";
import { wikiSearch } from "./wiki-search";

export { ASK_USER_TOOL } from "./ask-user-question";
export { SHOW_INSTANCE_CHANGES_TOOL } from "./show-instance-changes";
export { SHOW_MODPACK_TOOL } from "./show-modpack";

const BUILD_TOOL_BUILDERS = {
  search_base_modpacks: searchBaseModpacks,
  inspect_base_modpack: inspectBaseModpack,
  search_mods: () => searchMods(false),
  mod_get_detail: () => modGetDetail(false),
  resolve_mods: () => resolveMods(false),
  validate_modpack_plan: validateModpackPlan,
  build_modpack: buildModpack,
  show_modpack: showModpack,
  list_instances: listInstances,
  ask_user_question: askUserQuestion,
} as const;

const INSTANCE_TOOL_BUILDERS = {
  wiki_search: wikiSearch,
  wiki_open: wikiOpen,
  diagnose_instance: diagnoseInstance,
  start_deep_diagnosis: startDeepDiagnosis,
  run_diagnostic_trial: runDiagnosticTrial,
  finish_deep_diagnosis: finishDeepDiagnosis,
  search_mods: () => searchMods(true),
  mod_get_detail: () => modGetDetail(true),
  resolve_mods: () => resolveMods(true),
  show_instance_changes: showInstanceChanges,
  ask_user_question: askUserQuestion,
} as const;

export const BUILD_TOOL_NAMES = Object.keys(BUILD_TOOL_BUILDERS);
export const INSTANCE_TOOL_NAMES = Object.keys(INSTANCE_TOOL_BUILDERS);

function buildFrom(builders: Record<string, () => unknown>): ToolSet {
  return Object.fromEntries(
    Object.entries(builders).map(([name, build]) => [name, build()]),
  ) as ToolSet;
}

export function buildTools(mode: AgentModeInput = "build"): ToolSet {
  return normalizeAgentMode(mode) === "instance"
    ? buildFrom(INSTANCE_TOOL_BUILDERS)
    : buildFrom(BUILD_TOOL_BUILDERS);
}

export function toolSchemasForMode(mode: AgentModeInput): Record<string, z.ZodType> {
  return Object.fromEntries(
    Object.entries(buildTools(mode)).map(([name, value]) => [
      name,
      value.inputSchema as z.ZodType,
    ]),
  );
}

/**
 * Backward-compatible aggregate for callers that validate individual schemas
 * without a mode. Shared names use the build-profile schema; mode-aware hosts
 * should call `toolSchemasForMode`.
 */
export const toolSchemas: Record<string, z.ZodType> = Object.fromEntries(
  Object.entries({ ...buildTools("instance"), ...buildTools("build") }).map(([name, value]) => [
    name,
    value.inputSchema as z.ZodType,
  ]),
);
