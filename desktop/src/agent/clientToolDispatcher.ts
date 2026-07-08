import { commands } from "../ipc/bindings";
import { activeRoot } from "../store";
import type { AgentToolContext, AgentWikiContext } from "./chatStore";
import type { AgentMode } from "@kobemc/agent-core";

type SpectaResult<T> = { status: "ok"; data: T } | { status: "error"; error: string };

export async function unwrap<T>(p: Promise<SpectaResult<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

export const INTERACTIVE_CLIENT_TOOLS = new Set(["ask_user_question", "show_modpack"]);

export function isAutomaticClientTool(name: string): boolean {
  return !INTERACTIVE_CLIENT_TOOLS.has(name) && AUTO_TOOL_NAMES.has(name);
}

const AUTO_TOOL_NAMES = new Set([
  "search_base_modpacks",
  "inspect_base_modpack",
  "search_mods",
  "mod_get_detail",
  "resolve_mods",
  "build_modpack",
  "list_instances",
  "wiki_search",
  "wiki_open",
]);

const MODE_TOOL_NAMES: Record<AgentMode, Set<string>> = {
  modpack: new Set([
    "search_base_modpacks",
    "inspect_base_modpack",
    "search_mods",
    "mod_get_detail",
    "resolve_mods",
    "build_modpack",
    "list_instances",
  ]),
  wiki: new Set(["wiki_search", "wiki_open"]),
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
      return unwrap(commands.agentToolSearchMods(args as never));
    case "mod_get_detail":
      return unwrap(commands.agentToolModGetDetail(args as never));
    case "resolve_mods":
      return unwrap(commands.agentToolResolveMods(args as never));
    case "build_modpack":
      return unwrap(commands.agentToolBuildModpack(args as never));
    case "list_instances":
      return unwrap(commands.agentToolListInstances(activeRoot()));
    case "wiki_search":
      return unwrap(commands.agentToolWikiSearch(wikiRoot(context), wikiSearchArgs(args, context)));
    case "wiki_open":
      return unwrap(commands.agentToolWikiOpen(wikiRoot(context), wikiOpenArgs(args, context)));
    default:
      return Promise.reject(new Error(`unknown client tool: ${name}`));
  }
}

function assertToolAllowed(name: string, context: AgentToolContext | null): void {
  const mode = context?.mode ?? (context?.wiki ? "wiki" : "modpack");
  if (!MODE_TOOL_NAMES[mode].has(name)) {
    throw new Error(`${name} is not available in ${mode} agent mode`);
  }
}

function wikiContext(context: AgentToolContext | null): AgentWikiContext {
  const wiki = context?.wiki;
  if (!wiki || !wiki.modpackId || !wiki.instanceId || wiki.sourcePaths.length === 0) {
    throw new Error("wiki tools require an installed instance context");
  }
  return wiki;
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
