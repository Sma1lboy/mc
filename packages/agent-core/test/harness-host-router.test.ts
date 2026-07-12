import { describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import { createHarnessHostRouter } from "../bin/harness-host-router.mjs";

type Handler = (
  input: unknown,
  options: { toolCallId: string },
) => Promise<unknown>;

interface PendingAgentRun {
  history: UIMessage[];
  onUpdate: (message: UIMessage) => void;
  signal: AbortSignal;
  finish: (messages: UIMessage[], error?: string) => void;
}

function setup() {
  const sent: Array<Record<string, unknown>> = [];
  const createdAgents: Array<{
    conversationId: string;
    handlers: Record<string, Handler>;
    pending: PendingAgentRun[];
    dispose: ReturnType<typeof vi.fn>;
  }> = [];
  const agents = new Map<
    string,
    { handlers: Record<string, Handler>; pending: PendingAgentRun[]; dispose: ReturnType<typeof vi.fn> }
  >();
  const router = createHarnessHostRouter({
    send: (message) => sent.push(message),
    createAgent: (handlers, _options, conversationId) => {
      const pending: PendingAgentRun[] = [];
      const dispose = vi.fn(async () => {});
      createdAgents.push({ conversationId, handlers, pending, dispose });
      agents.set(conversationId, { handlers, pending, dispose });
      return {
        run: (
          history: UIMessage[],
          onUpdate: (message: UIMessage) => void,
          signal: AbortSignal,
        ) =>
          new Promise<{ messages: UIMessage[]; error?: string }>((resolve) => {
            pending.push({
              history,
              onUpdate,
              signal,
              finish: (messages, error) => resolve({ messages, error }),
            });
          }),
        dispose,
      };
    },
  });
  return { router, sent, agents, createdAgents };
}

function assistant(id: string, text: string): UIMessage {
  return { id, role: "assistant", parts: [{ type: "text", text }] };
}

describe("harness host router", () => {
  it("exposes confirmation tools instead of privileged action tools", async () => {
    const { router, createdAgents } = setup();
    router.handle({
      type: "turn",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      text: "prepare a plan",
      mode: "modpack",
    });

    await vi.waitFor(() => expect(createdAgents).toHaveLength(1));
    expect(createdAgents[0].handlers).toHaveProperty("confirm_modpack_build");
    expect(createdAgents[0].handlers).toHaveProperty("confirm_deep_diagnosis");
    expect(createdAgents[0].handlers).not.toHaveProperty("build_modpack");
    expect(createdAgents[0].handlers).not.toHaveProperty("start_deep_diagnosis");
  });

  it("starts a fresh Claude session when the provider session id changes", async () => {
    const { router, sent, createdAgents } = setup();
    router.handle({
      type: "turn",
      providerSessionId: "provider-session-1",
      conversationId: "A",
      runId: "run-1",
      text: "first Claude turn",
      mode: "modpack",
    });
    await vi.waitFor(() => expect(createdAgents).toHaveLength(1));
    const first = createdAgents[0].pending[0];
    first.finish(first.history);
    await vi.waitFor(() =>
      expect(sent).toContainEqual(expect.objectContaining({ type: "done", runId: "run-1" })),
    );

    router.handle({
      type: "turn",
      providerSessionId: "provider-session-2",
      conversationId: "A",
      runId: "run-2",
      text: "return to Claude after OpenRouter",
      mode: "modpack",
    });
    await vi.waitFor(() => expect(createdAgents).toHaveLength(2));
    expect(createdAgents[1].pending[0].history).toEqual([
      expect.objectContaining({
        role: "user",
        parts: [{ type: "text", text: "return to Claude after OpenRouter" }],
      }),
    ]);
  });

  it("runs A and B concurrently while rejecting overlapping turns only within A", async () => {
    const { router, sent, agents } = setup();
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "alpha", mode: "modpack" });
    router.handle({ type: "turn", providerSessionId: "session-B", conversationId: "B", runId: "run-B", text: "bravo", mode: "wiki" });
    await vi.waitFor(() => expect(agents.size).toBe(2));
    expect(agents.get("A")?.pending).toHaveLength(1);
    expect(agents.get("B")?.pending).toHaveLength(1);

    router.handle({ type: "turn", providerSessionId: "session-A2", conversationId: "A", runId: "run-A2", text: "too soon", mode: "wiki" });
    await vi.waitFor(() =>
      expect(sent).toContainEqual({
        type: "done",
        providerSessionId: "session-A2",
        conversationId: "A",
        runId: "run-A2",
        error: "turn already running",
      }),
    );

    const updateA = assistant("assistant-A", "A partial");
    agents.get("A")!.pending[0].onUpdate(updateA);
    expect(sent).toContainEqual({
      type: "update",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      message: updateA,
    });
    expect(sent).not.toContainEqual(expect.objectContaining({
      type: "update",
      conversationId: "B",
      message: updateA,
    }));

    agents.get("B")!.pending[0].finish(agents.get("B")!.pending[0].history);
    agents.get("A")!.pending[0].finish([...agents.get("A")!.pending[0].history, updateA]);
    await vi.waitFor(() =>
      expect(sent).toContainEqual({ type: "done", providerSessionId: "session-A", conversationId: "A", runId: "run-A" }),
    );
    expect(sent).toContainEqual({ type: "done", providerSessionId: "session-B", conversationId: "B", runId: "run-B" });
  });

  it("routes same-name tool calls and reverse-order results by real toolCallId", async () => {
    const { router, sent, agents } = setup();
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "ask twice", mode: "modpack" });
    await vi.waitFor(() => expect(agents.has("A")).toBe(true));
    const handler = agents.get("A")!.handlers.ask_user_question;

    const first = handler({ question: "first" }, { toolCallId: "call-1" });
    const second = handler({ question: "second" }, { toolCallId: "call-2" });
    expect(sent).toContainEqual({
      type: "tool_call",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "call-1",
      name: "ask_user_question",
      args: { question: "first" },
    });
    expect(sent).toContainEqual(expect.objectContaining({
      type: "tool_call",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "call-2",
    }));

    router.handle({
      type: "tool_result",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "call-2",
      ok: true,
      result: { selected: ["second"] },
    });
    await expect(second).resolves.toEqual({ selected: ["second"] });
    router.handle({
      type: "tool_result",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "call-1",
      ok: true,
      result: { selected: ["first"] },
    });
    await expect(first).resolves.toEqual({ selected: ["first"] });
  });

  it("aborts only the addressed conversation and run", async () => {
    const { router, agents } = setup();
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "alpha", mode: "modpack" });
    router.handle({ type: "turn", providerSessionId: "session-B", conversationId: "B", runId: "run-B", text: "bravo", mode: "modpack" });
    await vi.waitFor(() => expect(agents.size).toBe(2));

    router.handle({ type: "abort", providerSessionId: "session-A", conversationId: "A", runId: "run-A" });
    expect(agents.get("A")!.pending[0].signal.aborted).toBe(true);
    expect(agents.get("B")!.pending[0].signal.aborted).toBe(false);
  });
});
