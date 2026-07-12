import { describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import {
  AgentRunCoordinator,
  type AgentProviderRunRequest,
  type AgentProviderSession,
  type ConversationRunState,
} from "./runCoordinator";

function message(id: string, role: "user" | "assistant", text: string): UIMessage {
  return { id, role, parts: [{ type: "text", text }] };
}

interface PendingRun {
  request: AgentProviderRunRequest;
  finish: (messages: UIMessage[], error?: string) => void;
}

class DeferredSession implements AgentProviderSession {
  readonly pending: PendingRun[] = [];

  run(request: AgentProviderRunRequest): Promise<{ messages: UIMessage[]; error?: string }> {
    return new Promise((resolve) => {
      this.pending.push({
        request,
        finish: (messages, error) => resolve({ messages, error }),
      });
    });
  }
}

function harness() {
  const sessions = new Map<string, DeferredSession>();
  const states = new Map<string, ConversationRunState>();
  const automaticCalls: Array<{ name: string; input: unknown; context: unknown }> = [];
  let runSeq = 0;
  let messageSeq = 0;
  const coordinator = new AgentRunCoordinator({
    createProviderSession: async ({ conversationId }) => {
      const session = new DeferredSession();
      sessions.set(conversationId, session);
      return session;
    },
    isAutomaticTool: (name) => name === "inspect_instance",
    isInteractiveTool: (name) => name === "ask_user_question" || name === "show_modpack",
    runAutomaticTool: async (name, input, context) => {
      automaticCalls.push({ name, input, context });
      return { inspected: true };
    },
    onChange: (state) => states.set(state.id, state),
    makeRunId: () => `run-${++runSeq}`,
    makeMessageId: () => `msg-${++messageSeq}`,
  });
  return { coordinator, sessions, states, automaticCalls };
}

describe("AgentRunCoordinator", () => {
  it("stores provider-session startup failures on the owning conversation", async () => {
    const changes: ConversationRunState[] = [];
    let attempts = 0;
    const coordinator = new AgentRunCoordinator({
      createProviderSession: async () => {
        attempts += 1;
        if (attempts === 1) throw new Error("provider unavailable");
        return {
          run: async (request) => ({ messages: request.history }),
        };
      },
      isAutomaticTool: () => false,
      isInteractiveTool: () => false,
      runAutomaticTool: async () => null,
      onChange: (state) => changes.push(state),
      makeRunId: () => "run-1",
      makeMessageId: () => "msg-1",
    });
    coordinator.openConversation("A", { messages: [], toolContext: null });

    await expect(coordinator.sendMessage("A", "hello")).resolves.toBeUndefined();
    expect(coordinator.getConversation("A")).toMatchObject({
      streaming: false,
      error: "provider unavailable",
    });
    expect(changes.at(-1)?.id).toBe("A");

    await coordinator.sendMessage("A", "retry");
    expect(attempts).toBe(2);
    expect(coordinator.getConversation("A")).toMatchObject({ streaming: false, error: null });
  });

  it("runs different conversations concurrently without cross-writing updates", async () => {
    const { coordinator, sessions } = harness();
    coordinator.openConversation("A", { messages: [], toolContext: null });
    coordinator.openConversation("B", { messages: [], toolContext: null });

    const runA = coordinator.sendMessage("A", "alpha");
    const runB = coordinator.sendMessage("B", "bravo");
    await Promise.resolve();
    await Promise.resolve();

    const callA = sessions.get("A")?.pending[0];
    const callB = sessions.get("B")?.pending[0];
    expect(callA?.request.binding).toMatchObject({ conversationId: "A", runId: "run-1" });
    expect(callB?.request.binding).toMatchObject({ conversationId: "B", runId: "run-2" });

    const assistantA = message("assistant-A", "assistant", "A partial");
    const assistantB = message("assistant-B", "assistant", "B partial");
    callA?.request.onUpdate(assistantA);
    callB?.request.onUpdate(assistantB);

    expect(coordinator.getConversation("A").messages).toEqual([
      message("msg-1", "user", "alpha"),
      assistantA,
    ]);
    expect(coordinator.getConversation("B").messages).toEqual([
      message("msg-2", "user", "bravo"),
      assistantB,
    ]);
    expect(coordinator.getConversation("A").streaming).toBe(true);
    expect(coordinator.getConversation("B").streaming).toBe(true);

    callB?.finish([message("msg-2", "user", "bravo"), assistantB]);
    callA?.finish([message("msg-1", "user", "alpha"), assistantA]);
    await Promise.all([runA, runB]);

    expect(coordinator.getConversation("A").streaming).toBe(false);
    expect(coordinator.getConversation("B").streaming).toBe(false);
  });

  it("switches the selected conversation without cancelling a background run", async () => {
    const { coordinator, sessions } = harness();
    coordinator.openConversation("A", { messages: [], toolContext: null });
    coordinator.openConversation("B", { messages: [], toolContext: null });
    coordinator.selectConversation("A");

    const running = coordinator.sendMessage("A", "keep going");
    await Promise.resolve();
    await Promise.resolve();
    const call = sessions.get("A")!.pending[0];
    coordinator.selectConversation("B");

    expect(coordinator.selectedConversationId()).toBe("B");
    expect(call.request.signal.aborted).toBe(false);
    call.request.onUpdate(message("background-A", "assistant", "still running"));
    expect(coordinator.getConversation("A").messages).toContainEqual(
      message("background-A", "assistant", "still running"),
    );
    expect(coordinator.getConversation("B").messages).toEqual([]);

    call.finish([...call.request.history, message("background-A", "assistant", "still running")]);
    await running;
  });

  it("keeps one conversation sequential and drains queued messages in FIFO order", async () => {
    const { coordinator, sessions } = harness();
    coordinator.openConversation("A", { messages: [], toolContext: null });

    const first = coordinator.sendMessage("A", "first");
    await Promise.resolve();
    await Promise.resolve();
    await coordinator.sendMessage("A", "second");
    await coordinator.sendMessage("A", "third");

    expect(sessions.get("A")?.pending).toHaveLength(1);
    expect(coordinator.getConversation("A").queued).toEqual(["second", "third"]);

    const firstCall = sessions.get("A")!.pending[0];
    firstCall.finish(firstCall.request.history);
    await vi.waitFor(() => expect(sessions.get("A")?.pending).toHaveLength(2));
    expect(sessions.get("A")?.pending[1].request.history.at(-1)).toEqual(
      message("msg-2", "user", "second"),
    );

    const secondCall = sessions.get("A")!.pending[1];
    secondCall.finish(secondCall.request.history);
    await vi.waitFor(() => expect(sessions.get("A")?.pending).toHaveLength(3));
    expect(sessions.get("A")?.pending[2].request.history.at(-1)).toEqual(
      message("msg-3", "user", "third"),
    );

    const thirdCall = sessions.get("A")!.pending[2];
    thirdCall.finish(thirdCall.request.history);
    await first;
    expect(coordinator.getConversation("A").queued).toEqual([]);
    expect(coordinator.getConversation("A").streaming).toBe(false);
  });

  it("routes automatic tools with the run's frozen context after instance context changes", async () => {
    const { coordinator, sessions, automaticCalls } = harness();
    const originalContext = { mode: "wiki", wiki: { instanceId: "instance-A", sourcePaths: ["a"] } };
    coordinator.openConversation("A", { messages: [], toolContext: originalContext });

    const running = coordinator.sendMessage("A", "inspect it");
    await Promise.resolve();
    await Promise.resolve();
    const call = sessions.get("A")!.pending[0];
    originalContext.wiki.instanceId = "instance-B";
    originalContext.wiki.sourcePaths.push("b");

    const toolMessage = {
      id: "assistant-tool",
      role: "assistant",
      parts: [
        {
          type: "tool-inspect_instance",
          toolCallId: "tool-1",
          state: "input-available",
          input: { target: "current" },
        },
      ],
    } as UIMessage;
    call.finish([...call.request.history, toolMessage]);
    await vi.waitFor(() => expect(automaticCalls).toHaveLength(1));
    await vi.waitFor(() => expect(sessions.get("A")?.pending).toHaveLength(2));

    expect(automaticCalls).toEqual([
      {
        name: "inspect_instance",
        input: { target: "current" },
        context: { mode: "wiki", wiki: { instanceId: "instance-A", sourcePaths: ["a"] } },
      },
    ]);
    expect(Object.isFrozen(call.request.binding.toolContext)).toBe(true);
    expect(Object.isFrozen((call.request.binding.toolContext as typeof originalContext).wiki)).toBe(true);

    const resumed = sessions.get("A")!.pending[1];
    expect(resumed.request.history.at(-1)?.parts[0]).toMatchObject({
      toolCallId: "tool-1",
      state: "output-available",
      output: { inspected: true },
    });
    resumed.finish(resumed.request.history);
    await running;
  });

  it("does not resume OpenRouter while an interactive tool in the same response is pending", async () => {
    const { coordinator, sessions, automaticCalls } = harness();
    coordinator.openConversation("A", { messages: [], toolContext: null });
    const running = coordinator.sendMessage("A", "inspect then ask");
    await Promise.resolve();
    await Promise.resolve();
    const call = sessions.get("A")!.pending[0];
    const mixed = {
      id: "assistant-mixed",
      role: "assistant",
      parts: [
        {
          type: "tool-inspect_instance",
          toolCallId: "auto-1",
          state: "input-available",
          input: {},
        },
        {
          type: "tool-ask_user_question",
          toolCallId: "interactive-1",
          state: "input-available",
          input: { question: "continue?" },
        },
      ],
    } as UIMessage;
    call.finish([...call.request.history, mixed]);

    await vi.waitFor(() => expect(automaticCalls).toHaveLength(1));
    await vi.waitFor(() => expect(coordinator.getConversation("A").streaming).toBe(false));
    await running;
    expect(sessions.get("A")?.pending).toHaveLength(1);
    expect(coordinator.getConversation("A").messages.at(-1)?.parts).toEqual([
      expect.objectContaining({ toolCallId: "auto-1", state: "output-available" }),
      expect.objectContaining({ toolCallId: "interactive-1", state: "input-available" }),
    ]);
  });

  it("ignores callbacks only after their run is explicitly cancelled or superseded", async () => {
    const { coordinator, sessions } = harness();
    coordinator.openConversation("A", { messages: [], toolContext: null });

    const cancelled = coordinator.sendMessage("A", "cancel me");
    await Promise.resolve();
    await Promise.resolve();
    const cancelledCall = sessions.get("A")!.pending[0];
    coordinator.cancelConversation("A");
    expect(cancelledCall.request.signal.aborted).toBe(true);
    cancelledCall.request.onUpdate(message("late-cancelled", "assistant", "must be ignored"));
    cancelledCall.finish(cancelledCall.request.history);
    await cancelled;
    expect(coordinator.getConversation("A").messages).not.toContainEqual(
      message("late-cancelled", "assistant", "must be ignored"),
    );

    const oldRun = coordinator.sendMessage("A", "old run");
    await Promise.resolve();
    await Promise.resolve();
    const oldCall = sessions.get("A")!.pending[1];
    oldCall.finish(oldCall.request.history);
    await oldRun;
    oldCall.request.onUpdate(message("late-completed", "assistant", "still valid until superseded"));
    expect(coordinator.getConversation("A").messages).toContainEqual(
      message("late-completed", "assistant", "still valid until superseded"),
    );
    const replacement = coordinator.sendMessage("A", "replacement");
    await Promise.resolve();
    await Promise.resolve();
    oldCall.request.onUpdate(message("late-old", "assistant", "must also be ignored"));
    expect(coordinator.getConversation("A").messages).not.toContainEqual(
      message("late-old", "assistant", "must also be ignored"),
    );
    const replacementCall = sessions.get("A")!.pending[2];
    replacementCall.request.onUpdate(message("valid-new", "assistant", "accepted"));
    expect(coordinator.getConversation("A").messages).toContainEqual(
      message("valid-new", "assistant", "accepted"),
    );
    replacementCall.finish([
      ...replacementCall.request.history,
      message("valid-new", "assistant", "accepted"),
    ]);
    await replacement;
  });

  it("does not start the next turn in a conversation until a cancelled provider run settles", async () => {
    const { coordinator, sessions } = harness();
    coordinator.openConversation("A", { messages: [], toolContext: null });

    const cancelled = coordinator.sendMessage("A", "cancel me");
    await Promise.resolve();
    await Promise.resolve();
    const cancelledCall = sessions.get("A")!.pending[0];
    coordinator.cancelConversation("A");
    await coordinator.sendMessage("A", "after cancellation");

    expect(sessions.get("A")?.pending).toHaveLength(1);
    expect(coordinator.getConversation("A").queued).toEqual(["after cancellation"]);

    cancelledCall.finish(cancelledCall.request.history);
    await vi.waitFor(() => expect(sessions.get("A")?.pending).toHaveLength(2));
    const nextCall = sessions.get("A")!.pending[1];
    nextCall.finish(nextCall.request.history);
    await cancelled;
  });

  it("resolves same-name interactive calls independently by toolCallId", async () => {
    const { coordinator, sessions } = harness();
    coordinator.openConversation("A", { messages: [], toolContext: null });
    const running = coordinator.sendMessage("A", "ask twice");
    await Promise.resolve();
    await Promise.resolve();
    const binding = sessions.get("A")!.pending[0].request.binding;

    const first = coordinator.waitForInteractiveTool(binding, "ask_user_question", "question-1");
    const second = coordinator.waitForInteractiveTool(binding, "ask_user_question", "question-2");
    expect(coordinator.getConversation("A").pendingInteractiveToolCallIds).toEqual([
      "question-1",
      "question-2",
    ]);

    coordinator.resolveInteractiveTool("A", binding.runId, "question-2", { selected: ["B"] });
    await expect(second).resolves.toEqual({ selected: ["B"] });
    expect(coordinator.getConversation("A").pendingInteractiveToolCallIds).toEqual(["question-1"]);

    coordinator.resolveInteractiveTool("A", binding.runId, "question-1", { selected: ["A"] });
    await expect(first).resolves.toEqual({ selected: ["A"] });
    expect(coordinator.getConversation("A").pendingInteractiveToolCallIds).toEqual([]);

    const call = sessions.get("A")!.pending[0];
    call.finish(call.request.history);
    await running;
  });
});
