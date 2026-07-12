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

    const runA = protocol.run(request("A", "run-A", { root: "/A" }, updateA), "modpack");
    const runB = protocol.run(request("B", "run-B", { root: "/B" }, updateB), "wiki");
    await vi.waitFor(() => expect(sent).toHaveLength(2));
    expect(sent).toContainEqual(expect.objectContaining({
      type: "turn",
      conversationId: "A",
      runId: "run-A",
      mode: "modpack",
    }));
    expect(sent).toContainEqual(expect.objectContaining({
      type: "turn",
      conversationId: "B",
      runId: "run-B",
      mode: "wiki",
    }));

    const answerA = assistant("answer-A", "A partial");
    protocol.handle({ type: "update", conversationId: "A", runId: "run-A", message: answerA });
    expect(updateA).toHaveBeenCalledWith(answerA);
    expect(updateB).not.toHaveBeenCalled();

    protocol.handle({ type: "done", conversationId: "B", runId: "run-B" });
    protocol.handle({ type: "done", conversationId: "A", runId: "run-A" });
    await expect(runA).resolves.toEqual({
      messages: [user("user-A", "A"), answerA],
      error: undefined,
    });
    await expect(runB).resolves.toEqual({ messages: [user("user-B", "B")], error: undefined });
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
    const running = protocol.run(runRequest, "modpack");
    await vi.waitFor(() => expect(sent).toHaveLength(1));

    protocol.handle({
      type: "tool_call",
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
      conversationId: "A",
      runId: "run-A",
      toolCallId: "question-1",
      name: "ask_user_question",
      args: { question: "first" },
    });
    protocol.handle({
      type: "tool_call",
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

    protocol.handle({ type: "done", conversationId: "A", runId: "run-A" });
    await running;
  });
});
