import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import type { AgentProviderRunRequest } from "./runCoordinator";

const fakeProviders = vi.hoisted(() => ({
  localFactoryCalls: [] as string[],
  runCalls: [] as AgentProviderRunRequest[],
  autoFinish: false,
  pending: [] as Array<{
    request: AgentProviderRunRequest;
    finish: (messages: UIMessage[], error?: string) => void;
  }>,
}));

const fakeRepository = vi.hoisted(() => ({
  save: vi.fn(),
  hydrate: vi.fn(async () => []),
  sync: vi.fn(async () => []),
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
    agentHistoryHydrate: vi.fn(async () => ({ status: "ok", data: [] })),
    agentHistorySync: vi.fn(async () => ({ status: "ok", data: [] })),
    agentHistorySave: vi.fn(async () => ({ status: "ok", data: null })),
    agentHistoryPut: vi.fn(async () => ({ status: "ok", data: null })),
    agentHistoryList: vi.fn(async () => ({ status: "ok", data: [] })),
    agentHistoryGet: vi.fn(async () => ({ status: "error", error: "missing" })),
  },
}));

vi.mock("../i18n", () => ({ t: (key: string) => key }));

vi.mock("./clientToolDispatcher", () => ({
  INTERACTIVE_CLIENT_TOOLS: new Set(["ask_user_question", "show_modpack"]),
  isAutomaticClientTool: (name: string) => name === "diagnose_instance",
  runLauncherClientTool: vi.fn(),
}));

vi.mock("./desktopAdapter", () => ({
  createDesktopAgent: vi.fn(async () => ({
    run: (request: AgentProviderRunRequest) => {
      fakeProviders.runCalls.push(request);
      if (fakeProviders.autoFinish) return Promise.resolve({ messages: request.history });
      return new Promise<{ messages: UIMessage[]; error?: string }>((resolve) => {
        fakeProviders.pending.push({
          request,
          finish: (messages, error) => resolve({ messages, error }),
        });
      });
    },
  })),
}));

vi.mock("./localRuntimeAdapter", () => ({
  createLocalRuntimeAgent: vi.fn(async (_mode: string, _hooks: unknown, providerSessionId: string) => {
    fakeProviders.localFactoryCalls.push(providerSessionId);
    return {
      run: (request: AgentProviderRunRequest) => {
        fakeProviders.runCalls.push(request);
        if (fakeProviders.autoFinish) return Promise.resolve({ messages: request.history });
        return new Promise<{ messages: UIMessage[]; error?: string }>((resolve) => {
          fakeProviders.pending.push({
            request,
            finish: (messages, error) => resolve({ messages, error }),
          });
        });
      },
    };
  }),
}));

vi.mock("./conversationRepository", () => ({
  conversationRepository: fakeRepository,
  mergeConversationRecords: (current: unknown[], incoming: unknown[]) => [...current, ...incoming],
}));

function assistant(id: string, text: string): UIMessage {
  return { id, role: "assistant", parts: [{ type: "text", text }] };
}

describe("chat store run isolation", () => {
  beforeEach(() => {
    fakeProviders.pending.length = 0;
    fakeProviders.localFactoryCalls.length = 0;
    fakeProviders.runCalls.length = 0;
    fakeProviders.autoFinish = false;
    fakeRepository.save.mockClear();
  });

  it("rejects oversized input before provider execution or persistence", async () => {
    const store = await import("./chatStore");
    store.newChat();
    store.resetAgent("openrouter");
    fakeProviders.autoFinish = true;
    const savesBefore = fakeRepository.save.mock.calls.length;

    await store.sendMessage("x".repeat(65_537));

    expect(fakeProviders.runCalls).toHaveLength(0);
    expect(fakeRepository.save.mock.calls).toHaveLength(savesBefore);
    expect(store.useChatStore.getState()).toMatchObject({
      messages: [],
      queued: [],
      streaming: false,
      error: "agent.inputTooLarge",
    });
  });

  it("uses the synchronously selected provider while settings persistence catches up", async () => {
    const store = await import("./chatStore");
    store.newChat();
    store.resetAgent("claude-code");

    const running = store.sendMessage("use Claude immediately");
    await vi.waitFor(() => expect(fakeProviders.localFactoryCalls).toHaveLength(1));
    expect(fakeProviders.localFactoryCalls[0]).toContain(store.currentChatSessionId());
    const call = fakeProviders.pending[0];
    call.finish(call.request.history);
    await running;
    store.resetAgent("openrouter");
  });

  it("offers a prior same-instance automatic output to the next conversation only", async () => {
    const store = await import("./chatStore");
    store.newChat();
    const toolContext = {
      root: "/game-root",
      mode: "instance" as const,
      instance: {
        root: "/game-root",
        modpackId: "pack-a",
        instanceId: "pack-a",
        sourcePaths: ["/game-root/versions/pack-a"],
        mcVersion: "1.20.1",
        loader: "fabric",
      },
    };
    store.useChatStore.setState({
      conversations: [{
        id: "prior-conversation",
        createdAt: 1,
        updatedAt: 2,
        title: "prior",
        toolContext,
        messages: [{
          id: "prior-assistant",
          role: "assistant",
          parts: [{
            type: "tool-diagnose_instance",
            toolCallId: "prior-diagnosis",
            state: "output-available",
            input: { mode: "inspect" },
            output: { status: "healthy" },
          }],
        } as UIMessage],
      }],
    });
    store.openAgentChat("check again", toolContext);
    store.resetAgent("openrouter");

    const running = store.sendMessage("is this instance still healthy?");
    await vi.waitFor(() => expect(fakeProviders.pending).toHaveLength(1));
    const call = fakeProviders.pending[0];
    expect(call.request.memory).toMatchObject({
      identity: {
        scopeId: 'instance:["/game-root","pack-a"]',
        conversationId: call.request.binding.conversationId,
      },
      candidates: [{
        conversationId: "prior-conversation",
        content: 'diagnose_instance output: {"status":"healthy"}',
      }],
    });
    expect(call.request.memory?.candidates).toHaveLength(1);

    call.finish(call.request.history);
    await running;
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
