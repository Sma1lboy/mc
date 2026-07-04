import { commands } from "../ipc/bindings";
import { activeRoot } from "../store";

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
]);

export function runLauncherClientTool(name: string, args: unknown): Promise<unknown> {
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
    default:
      return Promise.reject(new Error(`unknown client tool: ${name}`));
  }
}
