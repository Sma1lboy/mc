import { spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import {
  createHarnessHostLineReader,
  createHarnessHostRouter,
  HARNESS_HOST_TRANSPORT_LIMITS,
} from "../bin/harness-host-router.mjs";

const EXPECTED_MAX_FRAME_BYTES = HARNESS_HOST_TRANSPORT_LIMITS.maxFrameBytes;
const EXPECTED_MAX_JSON_DEPTH = HARNESS_HOST_TRANSPORT_LIMITS.maxJsonDepth;

type Handler = (
  input: unknown,
  options: { toolCallId: string },
) => Promise<unknown>;

interface PendingAgentRun {
  history: UIMessage[];
  onUpdate: (message: UIMessage) => void;
  signal: AbortSignal;
  options?: unknown;
  finish: (messages: UIMessage[], error?: string) => void;
}

function setup(sendOverride?: (message: Record<string, unknown>) => unknown) {
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
    send: (message) => {
      sent.push(message);
      return sendOverride?.(message);
    },
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
          options?: unknown,
        ) =>
          new Promise<{ messages: UIMessage[]; error?: string }>((resolve) => {
            pending.push({
              history,
              onUpdate,
              signal,
              options,
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

async function runHarnessHost(input: string): Promise<{
  code: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
}> {
  const host = spawn(
    process.execPath,
    [fileURLToPath(new URL("../bin/harness-host.mjs", import.meta.url))],
    { stdio: ["pipe", "pipe", "pipe"] },
  );
  let stdout = "";
  let stderr = "";
  host.stdout.setEncoding("utf8");
  host.stderr.setEncoding("utf8");
  host.stdout.on("data", (chunk) => { stdout += chunk; });
  host.stderr.on("data", (chunk) => { stderr += chunk; });
  host.stdin.on("error", () => {});

  await vi.waitFor(
    () => expect(stderr).toContain("harness-host: ready"),
    { timeout: 5_000 },
  );
  host.stdin.end(`${input}\n{"type":"dispose"}\n`);
  const timeout = setTimeout(() => host.kill("SIGKILL"), 10_000);
  const [code, signal] = await once(host, "exit") as [number | null, NodeJS.Signals | null];
  clearTimeout(timeout);
  return { code, signal, stdout, stderr };
}

describe("harness host router", () => {
  it("passes desktop candidates to agent-core admission instead of injecting them directly", async () => {
    const { router, agents } = setup();
    const memory = {
      identity: { scopeId: 'instance:["/game","pack"]', conversationId: "A" },
      candidates: [{
        id: "candidate",
        scopeId: 'instance:["/game","pack"]',
        conversationId: "old",
        visibility: "scope",
        tier: "recall",
        memoryKey: "automatic-tool:diagnose_instance",
        content: "diagnose_instance output: healthy",
        updatedAt: "2026-08-28T08:00:00.000Z",
        provenance: { source: "launcher", reference: "conversation=old" },
      }],
    };
    router.handle({
      type: "turn",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      text: "is it healthy?",
      mode: "instance",
      memory,
    });

    await vi.waitFor(() => expect(agents.get("A")?.pending).toHaveLength(1));
    expect(agents.get("A")?.pending[0].options).toEqual({ memory });
  });

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

  it("does not terminate the active run when its exact turn delivery is duplicated", async () => {
    const { router, sent, agents } = setup();
    const turn = {
      type: "turn",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      text: "alpha",
      mode: "modpack",
    };

    router.handle(turn);
    await vi.waitFor(() => expect(agents.get("A")?.pending).toHaveLength(1));
    router.handle(turn);
    await Promise.resolve();

    expect(sent).not.toContainEqual(expect.objectContaining({
      type: "done",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
    }));
    expect(agents.get("A")?.pending).toHaveLength(1);

    const active = agents.get("A")!.pending[0];
    active.finish(active.history);
    await vi.waitFor(() => expect(sent).toContainEqual({
      type: "done",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
    }));
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

  it("releases a tool identity when sending its call throws", async () => {
    let failNextToolSend = true;
    const { router, agents } = setup((message) => {
      if (message.type === "tool_call" && failNextToolSend) {
        failNextToolSend = false;
        throw new Error("tool call pipe closed");
      }
    });
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "ask", mode: "modpack" });
    await vi.waitFor(() => expect(agents.has("A")).toBe(true));
    const handler = agents.get("A")!.handlers.ask_user_question;

    await expect(
      handler({ question: "first attempt" }, { toolCallId: "call-1" }),
    ).rejects.toThrow("tool call pipe closed");

    const retry = handler({ question: "retry" }, { toolCallId: "call-1" });
    router.handle({
      type: "tool_result",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "call-1",
      ok: true,
      result: { selected: ["retry"] },
    });
    await expect(retry).resolves.toEqual({ selected: ["retry"] });
  });

  it("releases a tool identity when sending its call rejects", async () => {
    let failNextToolSend = true;
    const { router, agents } = setup((message) => {
      if (message.type === "tool_call" && failNextToolSend) {
        failNextToolSend = false;
        return Promise.reject(new Error("tool call pipe rejected"));
      }
    });
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "ask", mode: "modpack" });
    await vi.waitFor(() => expect(agents.has("A")).toBe(true));
    const handler = agents.get("A")!.handlers.ask_user_question;

    await expect(
      handler({ question: "first attempt" }, { toolCallId: "call-1" }),
    ).rejects.toThrow("tool call pipe rejected");

    const retry = handler({ question: "retry" }, { toolCallId: "call-1" });
    router.handle({
      type: "tool_result",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      toolCallId: "call-1",
      ok: true,
      result: { selected: ["retry"] },
    });
    await expect(retry).resolves.toEqual({ selected: ["retry"] });
  });

  it("does not let simultaneous conversations claim the same provider session", async () => {
    const { router, sent, agents, createdAgents } = setup();
    router.handle({ type: "turn", providerSessionId: "shared-session", conversationId: "A", runId: "run-A", text: "alpha", mode: "modpack" });
    router.handle({ type: "turn", providerSessionId: "shared-session", conversationId: "B", runId: "run-B", text: "bravo", mode: "modpack" });

    await vi.waitFor(() => expect(agents.get("A")?.pending).toHaveLength(1));
    await vi.waitFor(() => expect(sent).toContainEqual({
      type: "done",
      providerSessionId: "shared-session",
      conversationId: "B",
      runId: "run-B",
      error: "provider session belongs to another conversation",
    }));
    expect(createdAgents).toHaveLength(1);

    const active = agents.get("A")!.pending[0];
    active.finish(active.history);
    await vi.waitFor(() => expect(sent).toContainEqual({
      type: "done",
      providerSessionId: "shared-session",
      conversationId: "A",
      runId: "run-A",
    }));
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

  it("aborts active runs and suppresses their late output during dispose", async () => {
    const { router, sent, agents } = setup();
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "alpha", mode: "modpack" });
    await vi.waitFor(() => expect(agents.has("A")).toBe(true));
    const run = agents.get("A")!.pending[0];

    await router.dispose();
    const sentAtDispose = sent.length;
    expect.soft(run.signal.aborted).toBe(true);

    run.onUpdate(assistant("late-update", "must not escape shutdown"));
    run.finish(run.history);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(sent.slice(sentAtDispose)).toEqual([]);
  });

  it("does not start a turn when dispose wins the session setup race", async () => {
    const { router, sent, createdAgents } = setup();
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "alpha", mode: "modpack" });

    await router.dispose();
    await Promise.resolve();

    expect(createdAgents).toHaveLength(1);
    expect(createdAgents[0].pending).toEqual([]);
    expect(sent).toEqual([]);
  });

  it("rejects pending tool calls when their host session is disposed", async () => {
    const { router, agents } = setup();
    router.handle({ type: "turn", providerSessionId: "session-A", conversationId: "A", runId: "run-A", text: "ask", mode: "modpack" });
    await vi.waitFor(() => expect(agents.has("A")).toBe(true));
    const agent = agents.get("A")!;
    const rejection = vi.fn();
    void agent.handlers.ask_user_question(
      { question: "still there?" },
      { toolCallId: "call-A" },
    ).catch(rejection);

    await router.dispose();
    await Promise.resolve();

    expect(rejection).toHaveBeenCalledWith(expect.objectContaining({ message: "router disposed" }));
    agent.pending[0].finish(agent.pending[0].history);
  });
});

describe("harness host JSON-line admission", () => {
  it("preserves chunked valid frames and memory routing", async () => {
    const { router, agents } = setup();
    const fatals: Error[] = [];
    const badLines: string[] = [];
    const reader = createHarnessHostLineReader({
      onMessage: (message) => router.handle(message),
      onBadLine: (line) => badLines.push(line),
      onFatal: (error) => fatals.push(error),
    });
    const memory = {
      identity: { scopeId: 'instance:["/game","pack"]', conversationId: "A" },
      candidates: [],
    };
    const frame = JSON.stringify({
      type: "turn",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
      text: "remember safely",
      mode: "instance",
      memory,
    });

    reader.push(Buffer.from(frame.slice(0, 17)));
    reader.push(Buffer.from(`${frame.slice(17)}\r\n`));

    await vi.waitFor(() => expect(agents.get("A")?.pending).toHaveLength(1));
    expect(agents.get("A")?.pending[0].options).toEqual({ memory });
    expect({ fatals, badLines }).toEqual({ fatals: [], badLines: [] });
  });

  it("keeps admitted cancellation isolated across frames", async () => {
    const { router, agents } = setup();
    const reader = createHarnessHostLineReader({
      onMessage: (message) => router.handle(message),
      onBadLine: vi.fn(),
      onFatal: vi.fn(),
    });
    const turn = (id: string) => JSON.stringify({
      type: "turn",
      providerSessionId: `session-${id}`,
      conversationId: id,
      runId: `run-${id}`,
      text: id,
      mode: "instance",
    });
    reader.push(Buffer.from(`${turn("A")}\n${turn("B")}\n`));
    await vi.waitFor(() => expect(agents.size).toBe(2));

    reader.push(Buffer.from(`${JSON.stringify({
      type: "abort",
      providerSessionId: "session-A",
      conversationId: "A",
      runId: "run-A",
    })}\n`));

    expect(agents.get("A")!.pending[0].signal.aborted).toBe(true);
    expect(agents.get("B")!.pending[0].signal.aborted).toBe(false);
  });

  it("terminates safely before parsing an oversized memory frame", async () => {
    const frame = JSON.stringify({
      type: "turn",
      memory: { candidates: [{ content: "x".repeat(EXPECTED_MAX_FRAME_BYTES) }] },
    });

    const result = await runHarnessHost(frame);

    expect(result).toMatchObject({ code: 1, signal: null, stdout: "" });
    expect(result.stderr).toContain("FRAME_TOO_LARGE");
  });

  it("terminates safely before parsing excessive JSON nesting", async () => {
    const nested = `${"[".repeat(EXPECTED_MAX_JSON_DEPTH + 1)}0${"]".repeat(EXPECTED_MAX_JSON_DEPTH + 1)}`;
    const frame = `{"type":"turn","payload":${nested}}`;

    const result = await runHarnessHost(frame);

    expect(result).toMatchObject({ code: 1, signal: null, stdout: "" });
    expect(result.stderr).toContain("JSON_DEPTH_EXCEEDED");
  });
});
