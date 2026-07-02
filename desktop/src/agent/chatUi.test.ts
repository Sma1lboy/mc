import { describe, expect, it } from "vitest";

import { agentUiKeys } from "./chatUi";

describe("agentUiKeys", () => {
  it("uses build copy by default", () => {
    expect(agentUiKeys().title).toBe("agent.title");
    expect(agentUiKeys().placeholder).toBe("agent.placeholder");
  });

  it("uses wiki copy for wiki profile", () => {
    expect(agentUiKeys("wiki")).toEqual({
      title: "agent.wikiTitle",
      subtitle: "agent.wikiSubtitle",
      placeholder: "agent.wikiPlaceholder",
      emptyTitle: "agent.wikiEmptyTitle",
      emptyHint: "agent.wikiEmptyHint",
    });
  });
});
