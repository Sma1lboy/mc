// Desktop host adapter for the modpack-agent brain.
//
// Reads the LLM settings from `agent_llm_config` and creates the OpenRouter
// agent. Tool execution is NOT here: agent-core emits client-side tool calls,
// and chatStore dispatches them through the launcher/Rust IPC boundary.
// Loaded lazily by chatStore (dynamic import) so `ai` + provider never enter the
// main bundle for rust-brain users.

import { commands } from "../ipc/bindings";
import { createModpackAgent, type ModpackAgent } from "@kobemc/agent-core";
import type { AgentLlmSettings, AgentMode } from "@kobemc/agent-core";
import { unwrap } from "./clientToolDispatcher";

async function loadSettings(): Promise<AgentLlmSettings> {
  const dto = await unwrap(commands.agentLlmConfig());
  return { apiKey: dto.api_key, model: dto.model, baseUrl: dto.base_url };
}

/** Create a desktop-hosted modpack agent (LLM config + Tauri tool backend). */
export async function createDesktopAgent(mode: AgentMode = "modpack"): Promise<ModpackAgent> {
  const settings = await loadSettings();
  return createModpackAgent(settings, { mode });
}
