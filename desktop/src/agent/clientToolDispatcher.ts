import { commands } from "../ipc/bindings";
import { activeRoot } from "../store";
import type { AgentInstanceContext, AgentToolContext, AgentWikiContext } from "./chatStore";
import type { AgentMode } from "@kobemc/agent-core";

type SpectaResult<T> = { status: "ok"; data: T } | { status: "error"; error: string };

export async function unwrap<T>(p: Promise<SpectaResult<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

export const INTERACTIVE_CLIENT_TOOLS = new Set([
  "ask_user_question",
  "show_modpack",
  "show_instance_changes",
]);

export function isAutomaticClientTool(name: string): boolean {
  return !INTERACTIVE_CLIENT_TOOLS.has(name) && AUTO_TOOL_NAMES.has(name);
}

const AUTO_TOOL_NAMES = new Set([
  "search_base_modpacks",
  "inspect_base_modpack",
  "search_mods",
  "mod_get_detail",
  "resolve_mods",
  "validate_modpack_plan",
  "build_modpack",
  "list_instances",
  "diagnose_instance",
  "start_deep_diagnosis",
  "run_diagnostic_trial",
  "finish_deep_diagnosis",
  "wiki_search",
  "wiki_open",
]);

const MODE_TOOL_NAMES: Record<AgentMode, Set<string>> = {
  build: new Set([
    "search_base_modpacks",
    "inspect_base_modpack",
    "search_mods",
    "mod_get_detail",
    "resolve_mods",
    "validate_modpack_plan",
    "build_modpack",
    "list_instances",
  ]),
  instance: new Set([
    "wiki_search",
    "wiki_open",
    "diagnose_instance",
    "start_deep_diagnosis",
    "run_diagnostic_trial",
    "finish_deep_diagnosis",
    "search_mods",
    "mod_get_detail",
    "resolve_mods",
  ]),
};

export function runLauncherClientTool(
  name: string,
  args: unknown,
  context: AgentToolContext | null = null,
): Promise<unknown> {
  assertToolAllowed(name, context);
  switch (name) {
    case "search_base_modpacks":
      return unwrap(commands.agentToolSearchBaseModpacks(args as never));
    case "inspect_base_modpack":
      return unwrap(commands.agentToolInspectBaseModpack(args as never));
    case "search_mods":
      return unwrap(commands.agentToolSearchMods(targetedSearchArgs(args, context)));
    case "mod_get_detail":
      return unwrap(commands.agentToolModGetDetail(targetedDetailArgs(args, context)));
    case "resolve_mods":
      return unwrap(commands.agentToolResolveMods(targetedResolveArgs(args, context)));
    case "validate_modpack_plan":
      return unwrap(commands.agentToolValidateModpackPlan(args as never));
    case "build_modpack":
      return unwrap(commands.agentToolBuildModpack(args as never));
    case "list_instances":
      return unwrap(commands.agentToolListInstances(activeRoot()));
    case "diagnose_instance": {
      const instance = instanceContext(context);
      return unwrap(
        commands.agentToolDiagnoseInstance(
          instance.root || activeRoot(),
          instance.instanceId,
          diagnoseArgs(args),
        ),
      );
    }
    case "start_deep_diagnosis": {
      const instance = instanceContext(context);
      return unwrap(
        commands.agentToolStartDeepDiagnosis(
          instance.root || activeRoot(),
          instance.instanceId,
        ),
      );
    }
    case "run_diagnostic_trial": {
      const instance = instanceContext(context);
      return unwrap(
        commands.agentToolRunDiagnosticTrial(
          instance.root || activeRoot(),
          instance.instanceId,
          args as never,
        ),
      );
    }
    case "finish_deep_diagnosis": {
      const instance = instanceContext(context);
      return unwrap(
        commands.agentToolFinishDeepDiagnosis(
          instance.root || activeRoot(),
          instance.instanceId,
          args as never,
        ),
      );
    }
    case "wiki_search":
      return unwrap(commands.agentToolWikiSearch(wikiRoot(context), wikiSearchArgs(args, context)));
    case "wiki_open":
      return unwrap(commands.agentToolWikiOpen(wikiRoot(context), wikiOpenArgs(args, context)));
    default:
      return Promise.reject(new Error(`unknown client tool: ${name}`));
  }
}

function assertToolAllowed(name: string, context: AgentToolContext | null): void {
  const mode = contextMode(context);
  if (!MODE_TOOL_NAMES[mode].has(name)) {
    throw new Error(`${name} is not available in ${mode} agent mode`);
  }
}

function wikiContext(context: AgentToolContext | null): AgentWikiContext {
  const wiki = context?.instance ?? context?.wiki;
  if (!wiki || !wiki.modpackId || !wiki.instanceId || wiki.sourcePaths.length === 0) {
    throw new Error("wiki tools require an installed instance context");
  }
  return wiki;
}

function instanceContext(context: AgentToolContext | null): AgentInstanceContext {
  const instance = context?.instance;
  if (
    !instance ||
    !instance.instanceId ||
    !instance.mcVersion ||
    !instance.loader ||
    instance.sourcePaths.length === 0
  ) {
    throw new Error("instance tools require a bound installed instance context");
  }
  return instance;
}

function isInstanceMode(context: AgentToolContext | null): boolean {
  return contextMode(context) === "instance";
}

function contextMode(context: AgentToolContext | null): AgentMode {
  if (context?.mode === "instance" || context?.mode === "wiki") return "instance";
  if (context?.mode === "build" || context?.mode === "modpack") return "build";
  return context?.instance || context?.wiki ? "instance" : "build";
}

function targetedSearchArgs(args: unknown, context: AgentToolContext | null): never {
  if (!isInstanceMode(context)) return args as never;
  const instance = instanceContext(context);
  const input = objectArgs(args);
  return {
    query: requiredString(input.query, "search_mods requires query"),
    mc_version: instance.mcVersion,
    loader: instance.loader,
  } as never;
}

function targetedDetailArgs(args: unknown, context: AgentToolContext | null): never {
  if (!isInstanceMode(context)) return args as never;
  const instance = instanceContext(context);
  const input = objectArgs(args);
  const out: Record<string, unknown> = {
    project_id: requiredString(input.project_id, "mod_get_detail requires project_id"),
    minecraft_version: instance.mcVersion,
    loader: instance.loader,
  };
  if (typeof input.provider === "string") out.provider = input.provider;
  return out as never;
}

function targetedResolveArgs(args: unknown, context: AgentToolContext | null): never {
  if (!isInstanceMode(context)) return args as never;
  const instance = instanceContext(context);
  const input = objectArgs(args);
  if (!Array.isArray(input.project_ids) || input.project_ids.some((id) => typeof id !== "string")) {
    throw new Error("resolve_mods requires project_ids");
  }
  return {
    project_ids: input.project_ids,
    mc_version: instance.mcVersion,
    loader: instance.loader,
  } as never;
}

function diagnoseArgs(args: unknown): never {
  const input = objectArgs(args);
  return {
    include_log_tail: input.include_log_tail === true,
  } as never;
}

function wikiRoot(context: AgentToolContext | null): string {
  return wikiContext(context).root || activeRoot();
}

function wikiSearchArgs(args: unknown, context: AgentToolContext | null): never {
  const wiki = wikiContext(context);
  const input = objectArgs(args);
  const query = input.query;
  if (typeof query !== "string" || !query.trim()) throw new Error("wiki_search requires query");
  const out: Record<string, unknown> = {
    modpack_id: wiki.modpackId,
    instance_id: wiki.instanceId,
    source_paths: wiki.sourcePaths,
    query,
  };
  if (typeof input.top_k === "number") out.top_k = input.top_k;
  if (typeof input.kind === "string") out.kind = input.kind;
  if (typeof input.target_id === "string") out.target_id = input.target_id;
  if (typeof input.ingredient_id === "string") out.ingredient_id = input.ingredient_id;
  if (typeof input.include_structured === "boolean") out.include_structured = input.include_structured;
  return out as never;
}

function wikiOpenArgs(args: unknown, context: AgentToolContext | null): never {
  const wiki = wikiContext(context);
  const input = objectArgs(args);
  const chunkId = input.chunk_id;
  if (typeof chunkId !== "string" || !chunkId.trim()) throw new Error("wiki_open requires chunk_id");
  return {
    modpack_id: wiki.modpackId,
    instance_id: wiki.instanceId,
    source_paths: wiki.sourcePaths,
    chunk_id: chunkId,
  } as never;
}

function objectArgs(args: unknown): Record<string, unknown> {
  return args && typeof args === "object" ? (args as Record<string, unknown>) : {};
}

function requiredString(value: unknown, message: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(message);
  return value;
}
