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

import { loadConversation, modpackWikiPrompt, newChat, openAgentChat, sendMessage, useChatStore } from "./chatStore";
import { useAppStore } from "../store";

const WIKI_CONTEXT: AgentToolContext = {
  profile: "wiki",
  wiki: {
    modpackId: "better-mc",
    instanceId: "local-instance",
    sourcePaths: ["/tmp/better-mc"],
  },
};
const OTHER_WIKI_CONTEXT: AgentToolContext = {
  profile: "wiki",
  wiki: {
    modpackId: "sky-factory",
    instanceId: "other-instance",
    sourcePaths: ["/tmp/sky-factory"],
  },
};

function messageTexts(): string[] {
  return useChatStore
    .getState()
    .messages.flatMap((m) => m.parts.map((p) => (p.type === "text" ? p.text : "")).filter(Boolean));
}

function resetStores(): void {
  const buildWindow = {
    convId: "test-build",
    messages: [],
    streaming: false,
    queued: [],
    error: null,
    draft: null,
    toolContext: null,
  };
  useChatStore.setState({
    windowKey: "build",
    windows: { build: buildWindow },
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

    expect(useChatStore.getState().windowKey).toBe("wiki:local-instance");
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
    openAgentChat("How do I open the Aether portal?", WIKI_CONTEXT);

    newChat();

    expect(useChatStore.getState().windowKey).toBe("build");
    expect(useChatStore.getState().toolContext).toBeNull();
  });

  it("can keep host tool context when starting a scoped new chat", () => {
    openAgentChat("How do I open the Aether portal?", WIKI_CONTEXT);

    newChat({ preserveToolContext: true });

    expect(useChatStore.getState().windowKey).toBe("wiki:local-instance");
    expect(useChatStore.getState().toolContext).toEqual(WIKI_CONTEXT);
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

  it("keeps build and wiki entry sessions isolated", async () => {
    openAgentChat("Build me a tech pack");
    await sendMessage("Build me a tech pack");

    openAgentChat("How do I open the Aether portal?", WIKI_CONTEXT);

    expect(useChatStore.getState().windowKey).toBe("wiki:local-instance");
    expect(useChatStore.getState().messages).toEqual([]);
    expect(useChatStore.getState().toolContext).toEqual(WIKI_CONTEXT);

    await sendMessage("How do I open the Aether portal?");

    openAgentChat("Build another pack");

    expect(useChatStore.getState().windowKey).toBe("build");
    expect(useChatStore.getState().toolContext).toBeNull();
    expect(messageTexts()).toEqual(["Build me a tech pack"]);
  });

  it("keeps wiki sessions isolated per modpack scope", async () => {
    openAgentChat("Better MC wiki", WIKI_CONTEXT);
    await sendMessage("Aether portal?");

    openAgentChat("Sky Factory wiki", OTHER_WIKI_CONTEXT);

    expect(useChatStore.getState().windowKey).toBe("wiki:other-instance");
    expect(useChatStore.getState().messages).toEqual([]);

    await sendMessage("How do I get dirt?");
    openAgentChat("Better MC wiki again", WIKI_CONTEXT);

    expect(useChatStore.getState().windowKey).toBe("wiki:local-instance");
    expect(useChatStore.getState().toolContext).toEqual(WIKI_CONTEXT);
    expect(messageTexts()).toEqual(["Aether portal?"]);
  });

  it("builds a modpack wiki entry prompt that directly starts wiki_search", () => {
    const prompt = modpackWikiPrompt("Better MC");

    expect(prompt).toContain("Better MC");
    expect(prompt).toContain("wiki_search");
  });
});
