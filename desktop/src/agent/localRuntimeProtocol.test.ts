import { describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import {
  createLocalRuntimeProtocol,
  type LocalRuntimeOutboundMessage,
} from "./localRuntimeProtocol";
import type {
  AgentProviderRunRequest,
  AgentProviderSession,
  AgentRunBinding,
} from "./runCoordinator";

function user(id: string, text: string): UIMessage {
  return { id, role: "user", parts: [{ type: "text", text }] };
}

function assistant(id: string, text: string): UIMessage {
  return { id, role: "assistant", parts: [{ type: "text", text }] };
}

function request(
  conversationId: string,
  runId: string,
  context: unknown,
  onUpdate = vi.fn(),
): AgentProviderRunRequest {
  const providerSession = {} as AgentProviderSession;
  const abortController = new AbortController();
  const binding: AgentRunBinding = Object.freeze({
    conversationId,
    runId,
    providerSession,
    toolContext: context,
    abortController,
  });
  return {
    binding,
    history: [user(`user-${conversationId}`, conversationId)],
    onUpdate,
    signal: abortController.signal,
  };
}

describe("local runtime protocol", () => {
  it("routes interleaved update and done events to their exact conversation and run", async () => {
    const sent: LocalRuntimeOutboundMessage[] = [];
    const updateA = vi.fn();
    const updateB = vi.fn();
    const protocol = createLocalRuntimeProtocol({
      send: async (message) => void sent.push(message),
      isInteractiveTool: () => false,
      runAutomaticTool: async () => null,
      waitForInteractiveTool: async () => null,
    });

    const runA = protocol.run(request("A", "run-A", { root: "/A" }, updateA), "build", "session-A");
    const runB = protocol.run(request("B", "run-B", { root: "/B" }, updateB), "instance", "session-B");
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect(sent).toContainEqual(expect.objectContaining({
      type: "turn",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      mode: "build",
    }));
    expect(sent).toContainEqual(expect.objectContaining({
      type: "turn",
      providerSessionId: "session-B",
      conversationId: "B",
      runId: "run-B",
      mode: "instance",
    }));

    const answerA = assistant("answer-A", "A partial");
    protocol.handle({ type: "update", providerSessionId: "session-A", conversationId: "A", runId: "run-A", message: answerA });
    expect(updateA).toHaveBeenCalledWith(answerA);
    expect(updateB).not.toHaveBeenCalled();

    protocol.handle({
      type: "done",
      providerSessionId: "session-B",
      conversationId: "B",
      runId: "run-B",
      promptVersion: "instance-agent-test",
    });
    protocol.handle({ type: "done", providerSessionId: "session-A", conversationId: "A", runId: "run-A" });
    await expect(runA).resolves.toEqual({
      messages: [user("user-A", "A"), answerA],
      error: undefined,
    });
    await expect(runB).resolves.toEqual({
      messages: [user("user-B", "B")],
      error: undefined,
      promptVersion: "instance-agent-test",
    });
  });

  it("uses frozen run context and toolCallId for automatic and same-name interactive calls", async () => {
    const sent: LocalRuntimeOutboundMessage[] = [];
    const automatic = vi.fn(async () => ({ root: "A-result" }));
    const pending = new Map<string, (output: unknown) => void>();
    const protocol = createLocalRuntimeProtocol({
      send: async (message) => void sent.push(message),
      isInteractiveTool: (name) => name === "ask_user_question",
      runAutomaticTool: automatic,
      waitForInteractiveTool: (_binding, _name, toolCallId) =>
        new Promise((resolve) => pending.set(toolCallId, resolve)),
    });
    const runRequest = request("A", "run-A", Object.freeze({ root: "/instance-A" }));
    const running = protocol.run(runRequest, "build", "session-A");
    await vi.waitFor(() => expect(sent).toHaveLength(1));

    protocol.handle({
      type: "tool_call",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "auto-1",
      name: "list_instances",
      args: {},
    });
    await vi.waitFor(() => expect(automatic).toHaveBeenCalled());
    expect(automatic).toHaveBeenCalledWith("list_instances", {}, { root: "/instance-A" });

    protocol.handle({
      type: "tool_call",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "question-1",
      name: "ask_user_question",
      args: { question: "first" },
    });
    protocol.handle({
      type: "tool_call",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "question-2",
      name: "ask_user_question",
      args: { question: "second" },
    });
    await vi.waitFor(() => expect(pending.size).toBe(2));
    pending.get("question-2")?.({ selected: ["second"] });
    pending.get("question-1")?.({ selected: ["first"] });

    await vi.waitFor(() =>
      expect(sent).toContainEqual({
        type: "tool_result",
        providerSessionId: "session-A",
        conversationId: "A",
        runId: "run-A",
        toolCallId: "question-2",
        ok: true,
        result: { selected: ["second"] },
      }),
    );
    expect(sent).toContainEqual(expect.objectContaining({
      type: "tool_result",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "question-1",
    }));

    protocol.handle({ type: "done", providerSessionId: "session-A", conversationId: "A", runId: "run-A" });
    await running;
  });

  it("settles an aborted turn after a bounded grace period when the host never replies", async () => {
    vi.useFakeTimers();
    try {
      const sent: LocalRuntimeOutboundMessage[] = [];
      const onUpdate = vi.fn();
      const protocol = createLocalRuntimeProtocol({
        send: async (message) => void sent.push(message),
        isInteractiveTool: () => false,
        runAutomaticTool: async () => null,
        waitForInteractiveTool: async () => null,
        cancellationGraceMs: 25,
      });
      const runRequest = request("A", "run-A", null, onUpdate);
      const running = protocol.run(runRequest, "build", "session-A");
      let settled = false;
      void running.then(() => {
        settled = true;
      });
      await Promise.resolve();

      runRequest.binding.abortController.abort();
      expect(sent).toContainEqual({
        type: "abort",
        providerSessionId: "session-A",
        conversationId: "A",
        runId: "run-A",
      });
      await vi.advanceTimersByTimeAsync(25);
      await Promise.resolve();

      expect(settled).toBe(true);
      protocol.handle({
        type: "update",
        providerSessionId: "session-A",
        conversationId: "A",
        runId: "run-A",
        message: assistant("late", "ignored"),
      });
      expect(onUpdate).not.toHaveBeenCalled();
      await running;
    } finally {
      vi.useRealTimers();
    }
  });

  it("deduplicates tool calls and suppresses their result after the run terminates", async () => {
    const sent: LocalRuntimeOutboundMessage[] = [];
    let resolveAutomatic!: (result: unknown) => void;
    const automatic = vi.fn(
      () => new Promise<unknown>((resolve) => {
        resolveAutomatic = resolve;
      }),
    );
    const protocol = createLocalRuntimeProtocol({
      send: async (message) => void sent.push(message),
      isInteractiveTool: () => false,
      runAutomaticTool: automatic,
      waitForInteractiveTool: async () => null,
    });
    const running = protocol.run(request("A", "run-A", null), "build", "session-A");
    await Promise.resolve();
    const toolCall = {
      type: "tool_call" as const,
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "tool-1",
      name: "list_instances",
      args: {},
    };

    protocol.handle(toolCall);
    protocol.handle(toolCall);
    expect(automatic).toHaveBeenCalledTimes(1);

    protocol.handle({
      type: "done",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
    });
    await running;
    resolveAutomatic({ instances: [] });
    await Promise.resolve();
    await Promise.resolve();

    expect(sent.filter((message) => message.type === "tool_result")).toEqual([]);
  });

  it("cleans up the turn when sending a tool result fails", async () => {
    const protocol = createLocalRuntimeProtocol({
      send: async (message) => {
        if (message.type === "tool_result") throw new Error("tool result pipe closed");
      },
      isInteractiveTool: () => false,
      runAutomaticTool: async () => ({ instances: [] }),
      waitForInteractiveTool: async () => null,
    });
    const running = protocol.run(request("A", "run-A", null), "build", "session-A");
    await Promise.resolve();

    protocol.handle({
      type: "tool_call",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "tool-1",
      name: "list_instances",
      args: {},
    });

    await expect(running).resolves.toMatchObject({ error: "tool result pipe closed" });
  });

  it("converts a synchronous tool failure into one routed error result", async () => {
    const sent: LocalRuntimeOutboundMessage[] = [];
    const protocol = createLocalRuntimeProtocol({
      send: async (message) => void sent.push(message),
      isInteractiveTool: () => false,
      runAutomaticTool: () => {
        throw new Error("tool exploded");
      },
      waitForInteractiveTool: async () => null,
    });
    const running = protocol.run(request("A", "run-A", null), "build", "session-A");
    await Promise.resolve();

    expect(() => protocol.handle({
      type: "tool_call",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "tool-1",
      name: "list_instances",
      args: {},
    })).not.toThrow();
    await vi.waitFor(() => expect(sent).toContainEqual({
      type: "tool_result",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "tool-1",
      ok: false,
      error: "tool exploded",
    }));

    protocol.handle({
      type: "done",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
    });
    await running;
  });
});
