import {
  commands,
  type WikiOpenArgs,
  type WikiSearchArgs,
} from "../ipc/bindings";
import { activeRoot } from "../store";
import { t } from "../i18n";
import {
  rootFromAgentContext,
  type AgentInstanceContext,
  type AgentToolContext,
  type AgentWikiContext,
} from "./agentContext";
import type { AgentMode } from "@kobemc/agent-core";

type SpectaResult<T> = { status: "ok"; data: T } | { status: "error"; error: string };

export async function unwrap<T>(p: Promise<SpectaResult<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

export const INTERACTIVE_CLIENT_TOOLS = new Set([
  "ask_user_question",
  "confirm_modpack_build",
  "confirm_deep_diagnosis",
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
  "list_instances",
  "diagnose_instance",
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
    "list_instances",
  ]),
  instance: new Set([
    "wiki_search",
    "wiki_open",
    "diagnose_instance",
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
    case "list_instances":
      return unwrap(commands.agentToolListInstances(rootFromAgentContext(context, activeRoot)));
    case "diagnose_instance": {
      const instance = instanceContext(context);
      return unwrap(
        commands.agentToolDiagnoseInstance(
          instance.root,
          instance.instanceId,
          diagnoseArgs(args),
        ),
      );
    }
    case "run_diagnostic_trial": {
      const instance = instanceContext(context);
      return unwrap(
        commands.agentToolRunDiagnosticTrial(
          instance.root,
          instance.instanceId,
          args as never,
        ),
      );
    }
    case "finish_deep_diagnosis": {
      const instance = instanceContext(context);
      return unwrap(
        commands.agentToolFinishDeepDiagnosis(
          instance.root,
          instance.instanceId,
          args as never,
        ),
      );
    }
    case "wiki_search":
      return runWikiSearch(args, context);
    case "wiki_open":
      return runWikiOpen(args, context);
    default:
      return Promise.reject(new Error(t("agent.unknownClientTool", { name })));
  }
}

function assertToolAllowed(name: string, context: AgentToolContext | null): void {
  const mode = contextMode(context);
  if (!MODE_TOOL_NAMES[mode].has(name)) {
    throw new Error(t("agent.toolUnavailable", { name, mode }));
  }
}

function wikiContext(context: AgentToolContext | null): AgentWikiContext {
  const wiki = context?.instance ?? context?.wiki;
  if (!wiki || !wiki.modpackId || !wiki.instanceId) {
    throw new Error(t("agent.wikiContextRequired"));
  }
  return wiki;
}

function instanceContext(
  context: AgentToolContext | null,
): AgentInstanceContext & { root: string } {
  const instance = context?.instance;
  const root = context?.root ?? instance?.root;
  if (!instance || !root || !instance.instanceId || !instance.mcVersion || !instance.loader) {
    throw new Error(t("agent.instanceBindingRequired"));
  }
  return { ...instance, root };
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
    query: requiredString(input.query, t("agent.searchModsQueryRequired")),
    mc_version: instance.mcVersion,
    loader: instance.loader,
  } as never;
}

function targetedDetailArgs(args: unknown, context: AgentToolContext | null): never {
  if (!isInstanceMode(context)) return args as never;
  const instance = instanceContext(context);
  const input = objectArgs(args);
  const out: Record<string, unknown> = {
    project_id: requiredString(input.project_id, t("agent.modDetailProjectRequired")),
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
    throw new Error(t("agent.resolveModsProjectsRequired"));
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

interface ResolvedWikiContext {
  root: string;
  wiki: AgentWikiContext;
  sourcePaths: string[];
}

const resolvedInstanceDirs = new Map<string, Promise<string>>();

async function resolveWikiContext(context: AgentToolContext | null): Promise<ResolvedWikiContext> {
  const wiki = wikiContext(context);
  const root = context?.root ?? wiki.root;
  if (!root) {
    throw new Error(t("agent.restoredInstanceReopenRequired"));
  }
  const configured = wiki.sourcePaths?.filter((path) => path.trim()) ?? [];
  if (configured.length > 0) return { root, wiki, sourcePaths: configured };

  const key = JSON.stringify([root, wiki.instanceId]);
  let resolved = resolvedInstanceDirs.get(key);
  if (!resolved) {
    resolved = unwrap(commands.instanceDir(root, wiki.instanceId));
    resolvedInstanceDirs.set(key, resolved);
    void resolved.catch(() => resolvedInstanceDirs.delete(key));
  }
  return { root, wiki, sourcePaths: [await resolved] };
}

async function runWikiSearch(args: unknown, context: AgentToolContext | null): Promise<unknown> {
  const resolved = await resolveWikiContext(context);
  return unwrap(
    commands.agentToolWikiSearch(resolved.root, wikiSearchArgs(args, resolved.wiki, resolved.sourcePaths)),
  );
}

async function runWikiOpen(args: unknown, context: AgentToolContext | null): Promise<unknown> {
  const resolved = await resolveWikiContext(context);
  return unwrap(
    commands.agentToolWikiOpen(resolved.root, wikiOpenArgs(args, resolved.wiki, resolved.sourcePaths)),
  );
}

function wikiSearchArgs(
  args: unknown,
  wiki: AgentWikiContext,
  sourcePaths: string[],
): WikiSearchArgs {
  const input = objectArgs(args);
  const query = input.query;
  if (typeof query !== "string" || !query.trim()) {
    throw new Error(t("agent.wikiSearchQueryRequired"));
  }
  const out: WikiSearchArgs = {
    modpack_id: wiki.modpackId,
    instance_id: wiki.instanceId,
    source_paths: sourcePaths,
    query,
  };
  if (typeof input.top_k === "number") out.top_k = input.top_k;
  if (typeof input.kind === "string") out.kind = input.kind;
  if (typeof input.target_id === "string") out.target_id = input.target_id;
  if (typeof input.ingredient_id === "string") out.ingredient_id = input.ingredient_id;
  if (typeof input.include_structured === "boolean") out.include_structured = input.include_structured;
  return out;
}

function wikiOpenArgs(
  args: unknown,
  wiki: AgentWikiContext,
  sourcePaths: string[],
): WikiOpenArgs {
  const input = objectArgs(args);
  const chunkId = input.chunk_id;
  if (typeof chunkId !== "string" || !chunkId.trim()) {
    throw new Error(t("agent.wikiOpenChunkRequired"));
  }
  return {
    modpack_id: wiki.modpackId,
    instance_id: wiki.instanceId,
    source_paths: sourcePaths,
    chunk_id: chunkId,
  };
}

function objectArgs(args: unknown): Record<string, unknown> {
  return args && typeof args === "object" ? (args as Record<string, unknown>) : {};
}

function requiredString(value: unknown, message: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(message);
  return value;
}
