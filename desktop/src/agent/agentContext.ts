import type { AgentMode, AgentModeInput } from "@kobemc/agent-core";

export interface AgentWikiContext {
  root: string;
  modpackId: string;
  instanceId: string;
  sourcePaths: string[];
}

export interface AgentInstanceContext extends AgentWikiContext {
  mcVersion: string;
  loader: string;
}

export interface AgentToolContext {
  /** Launcher game root captured when the conversation/run is created. */
  root?: string;
  mode?: AgentModeInput;
  instance?: AgentInstanceContext;
  /** Legacy persisted wiki-only context. New instance entrypoints use `instance`. */
  wiki?: AgentWikiContext;
}

export function agentModeFromContext(context: AgentToolContext | null): AgentMode {
  const mode = context?.mode;
  if (mode === "instance" || mode === "wiki") return "instance";
  if (mode === "build" || mode === "modpack") return "build";
  return context?.instance || context?.wiki ? "instance" : "build";
}

export function rootFromAgentContext(
  context: AgentToolContext | null,
  fallback: () => string,
): string {
  return context?.root ?? context?.instance?.root ?? context?.wiki?.root ?? fallback();
}
