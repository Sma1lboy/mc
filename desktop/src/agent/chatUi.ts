import type { AgentProfile } from "@kobemc/agent-core";

export interface AgentUiKeys {
  title: string;
  subtitle: string;
  placeholder: string;
  emptyTitle: string;
  emptyHint: string;
}

const BUILD_UI_KEYS: AgentUiKeys = {
  title: "agent.title",
  subtitle: "agent.subtitle",
  placeholder: "agent.placeholder",
  emptyTitle: "agent.emptyTitle",
  emptyHint: "agent.emptyHint",
};

const WIKI_UI_KEYS: AgentUiKeys = {
  title: "agent.wikiTitle",
  subtitle: "agent.wikiSubtitle",
  placeholder: "agent.wikiPlaceholder",
  emptyTitle: "agent.wikiEmptyTitle",
  emptyHint: "agent.wikiEmptyHint",
};

export function agentUiKeys(profile: AgentProfile = "build"): AgentUiKeys {
  return profile === "wiki" ? WIKI_UI_KEYS : BUILD_UI_KEYS;
}
