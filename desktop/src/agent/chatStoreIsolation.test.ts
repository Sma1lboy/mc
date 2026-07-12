import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import type { AgentProviderRunRequest } from "./runCoordinator";

const fakeProviders = vi.hoisted(() => ({
  pending: [] as Array<{
    request: AgentProviderRunRequest;
    finish: (messages: UIMessage[], error?: string) => void;
  }>,
}));

vi.mock("../store", () => ({
  setCurrentPage: vi.fn(),
  activeRoot: () => "/game-root",
  kobeUser: () => null,
  useAppStore: { subscribe: vi.fn() },
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    getSettings: vi.fn(async () => ({ status: "ok", data: { agent_provider: null } })),
    agentHostStop: vi.fn(async () => ({ status: "ok", data: null })),
    agentHistoryPut: vi.fn(async () => ({ status: "ok", data: null })),
    agentHistoryList: vi.fn(async () => ({ status: "ok", data: [] })),
    agentHistoryGet: vi.fn(async () => ({ status: "error", error: "missing" })),
  },
}));

vi.mock("../i18n", () => ({ t: (key: string) => key }));

vi.mock("./clientToolDispatcher", () => ({
  INTERACTIVE_CLIENT_TOOLS: new Set(["ask_user_question", "show_modpack"]),
  isAutomaticClientTool: () => false,
  runLauncherClientTool: vi.fn(),
}));

vi.mock("./desktopAdapter", () => ({
  createDesktopAgent: vi.fn(async () => ({
    run: (request: AgentProviderRunRequest) =>
      new Promise<{ messages: UIMessage[]; error?: string }>((resolve) => {
        fakeProviders.pending.push({
          request,
          finish: (messages, error) => resolve({ messages, error }),
        });
      }),
  })),
}));

function assistant(id: string, text: string): UIMessage {
  return { id, role: "assistant", parts: [{ type: "text", text }] };
}

describe("chat store run isolation", () => {
  beforeEach(() => {
    fakeProviders.pending.length = 0;
  });

  it("keeps A running while new-chat selects and runs B, then restores A's background result", async () => {
    const store = await import("./chatStore");
    const conversationA = store.currentChatSessionId();
    const runA = store.sendMessage("alpha");
    await vi.waitFor(() => expect(fakeProviders.pending).toHaveLength(1));
    const callA = fakeProviders.pending[0];

    store.newChat();
    const conversationB = store.currentChatSessionId();
    expect(conversationB).not.toBe(conversationA);
    expect(callA.request.signal.aborted).toBe(false);

    const runB = store.sendMessage("bravo");
    await vi.waitFor(() => expect(fakeProviders.pending).toHaveLength(2));
    const callB = fakeProviders.pending[1];
    expect(callA.request.binding.conversationId).toBe(conversationA);
    expect(callB.request.binding.conversationId).toBe(conversationB);

    const answerA = assistant("answer-A", "A completed in background");
    callA.request.onUpdate(answerA);
    callA.finish([...callA.request.history, answerA]);
    const answerB = assistant("answer-B", "B completed");
    callB.finish([...callB.request.history, answerB]);
    await Promise.all([runA, runB]);

    store.loadConversation(conversationA);
    expect(store.useChatStore.getState().messages).toContainEqual(answerA);
    expect(store.useChatStore.getState().messages).not.toContainEqual(answerB);
  });

  it("routes a late interactive result to its captured conversation after the UI switches", async () => {
    const store = await import("./chatStore");
    store.newChat();
    const conversationA = store.currentChatSessionId();
    const runA = store.sendMessage("ask me");
    await vi.waitFor(() => expect(fakeProviders.pending).toHaveLength(1));
    const callA = fakeProviders.pending[0];
    const toolMessage = {
      id: "assistant-question",
      role: "assistant",
      parts: [
        {
          type: "tool-ask_user_question",
          toolCallId: "question-A",
          state: "input-available",
          input: { question: "continue?" },
        },
      ],
    } as UIMessage;
    callA.finish([...callA.request.history, toolMessage]);
    await runA;

    store.newChat();
    const conversationB = store.currentChatSessionId();
    store.resolveClientTool(
      conversationA,
      "assistant-question",
      "question-A",
      { selected: ["yes"] },
    );

    await vi.waitFor(() => expect(fakeProviders.pending).toHaveLength(2));
    expect(fakeProviders.pending[1].request.binding.conversationId).toBe(conversationA);
    expect(store.currentChatSessionId()).toBe(conversationB);
    expect(store.useChatStore.getState().messages).toEqual([]);

    fakeProviders.pending[1].finish(fakeProviders.pending[1].request.history);
  });
});
