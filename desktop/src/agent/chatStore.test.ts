import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentToolContext } from "@kobemc/agent-core";

const mocks = vi.hoisted(() => ({
  createDesktopAgent: vi.fn(async () => ({
    run: vi.fn(async (history: unknown[]) => ({ messages: history })),
  })),
}));

vi.mock("./desktopAdapter", () => ({
  createDesktopAgent: mocks.createDesktopAgent,
}));

import { loadConversation, newChat, openAgentChat, sendMessage, useChatStore } from "./chatStore";
import { useAppStore } from "../store";

const WIKI_CONTEXT: AgentToolContext = {
  wiki: {
    modpackId: "better-mc",
    instanceId: "local-instance",
    sourcePaths: ["/tmp/better-mc"],
  },
};

function resetStores(): void {
  useChatStore.setState({
    messages: [],
    streaming: false,
    queued: [],
    error: null,
    draft: null,
    toolContext: null,
    conversations: [],
  } as Partial<ReturnType<typeof useChatStore.getState>>);
  useAppStore.setState({ currentPage: "home" });
  mocks.createDesktopAgent.mockClear();
}

describe("chatStore agent context", () => {
  beforeEach(resetStores);

  it("stores optional host tool context with an agent draft", () => {
    openAgentChat("How do I open the Aether portal?", WIKI_CONTEXT);

    expect(useChatStore.getState().draft).toBe("How do I open the Aether portal?");
    expect(useChatStore.getState().toolContext).toEqual(WIKI_CONTEXT);
    expect(useAppStore.getState().currentPage).toBe("agent");
  });

  it("creates the desktop agent with the stored host tool context", async () => {
    openAgentChat("How do I open the Aether portal?", WIKI_CONTEXT);

    await sendMessage("How do I open the Aether portal?");

    expect(mocks.createDesktopAgent).toHaveBeenCalledWith(WIKI_CONTEXT);
  });

  it("clears host tool context when starting a new chat", () => {
    useChatStore.setState({ toolContext: WIKI_CONTEXT } as Partial<ReturnType<typeof useChatStore.getState>>);

    newChat();

    expect(useChatStore.getState().toolContext).toBeNull();
  });

  it("restores host tool context when loading a saved conversation", () => {
    useChatStore.setState({
      conversations: [
        {
          id: "chat-1",
          createdAt: 1,
          updatedAt: 2,
          title: "Aether portal",
          messages: [],
          toolContext: WIKI_CONTEXT,
        },
      ],
    } as Partial<ReturnType<typeof useChatStore.getState>>);

    loadConversation("chat-1");

    expect(useChatStore.getState().toolContext).toEqual(WIKI_CONTEXT);
  });
});
