import type { AgentMode } from "@kobemc/agent-core";

export interface AgentWikiContext {
  root: string;
  modpackId: string;
  instanceId: string;
  sourcePaths: string[];
}

export interface AgentToolContext {
  /** Launcher game root captured when the conversation/run is created. */
  root?: string;
  mode?: AgentMode;
  wiki?: AgentWikiContext;
}

export function agentModeFromContext(context: AgentToolContext | null): AgentMode {
  return context?.mode ?? (context?.wiki ? "wiki" : "modpack");
}

export function rootFromAgentContext(
  context: AgentToolContext | null,
  fallback: () => string,
): string {
  return context?.root ?? context?.wiki?.root ?? fallback();
}
