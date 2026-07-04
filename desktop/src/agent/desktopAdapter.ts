// Desktop host adapter for the modpack-agent brain.
//
// Builds the injected `ToolExecutor` from the six Tauri tool commands and reads
// the LLM settings from `agent_llm_config`. This is the ONLY place that touches
// Tauri (via the tauri-specta typed `commands`); `core/` stays host-agnostic.
// Loaded lazily by chatStore (dynamic import) so `ai` + provider never enter the
// main bundle for rust-brain users.

import { commands } from "../ipc/bindings";
import { activeRoot } from "../store";
import { createModpackAgent, type ModpackAgent } from "@kobemc/agent-core";
import type { AgentLlmSettings, ToolExecutor } from "@kobemc/agent-core";

// tauri-specta commands return { status:"ok",data } | { status:"error",error }.
type SpectaResult<T> = { status: "ok"; data: T } | { status: "error"; error: string };
async function unwrap<T>(p: Promise<SpectaResult<T>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw new Error(r.error);
  return r.data;
}

// Adapt a typed command into a ToolExecutor entry. The model's args are already
// zod-validated in core (and re-validated by Rust), so `args as A` is the single
// trust-boundary cast.
const bind =
  <A, T>(cmd: (a: A) => Promise<SpectaResult<T>>) =>
  (args: unknown): Promise<T> =>
    unwrap(cmd(args as A));

function buildExecutor(): ToolExecutor {
  return {
    search_base_modpacks: bind(commands.agentToolSearchBaseModpacks),
    inspect_base_modpack: bind(commands.agentToolInspectBaseModpack),
    search_mods: bind(commands.agentToolSearchMods),
    mod_get_detail: bind(commands.agentToolModGetDetail),
    resolve_mods: bind(commands.agentToolResolveMods),
    build_modpack: bind(commands.agentToolBuildModpack),
    // Launcher-side tool: needs the CURRENT game root, which only the UI knows —
    // injected per call (not captured at build time) so a root switch
    // mid-conversation is respected. (Installing is NOT here: it's the
    // `show_modpack` client tool — the user's click on the card, see ModpackCard.)
    list_instances: () => unwrap(commands.agentToolListInstances(activeRoot())),
  };
}

async function loadSettings(): Promise<AgentLlmSettings> {
  const dto = await unwrap(commands.agentLlmConfig());
  return { apiKey: dto.api_key, model: dto.model, baseUrl: dto.base_url };
}

/** Create a desktop-hosted modpack agent (LLM config + Tauri tool backend). */
export async function createDesktopAgent(): Promise<ModpackAgent> {
  const settings = await loadSettings();
  return createModpackAgent(settings, buildExecutor());
}
