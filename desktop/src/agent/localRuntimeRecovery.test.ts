import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import { createLocalRuntimeAgent } from "./localRuntimeAdapter";
import {
  AgentRunCoordinator,
  type AgentProviderRunRequest,
  type AgentProviderSession,
  type AgentRunBinding,
} from "./runCoordinator";
import type {
  LocalRuntimeInboundMessage,
  LocalRuntimeOutboundMessage,
} from "./localRuntimeProtocol";

type HostListener = (event: { payload: { line: string } }) => void;

const host = vi.hoisted(() => ({
  listeners: [] as HostListener[],
  sends: vi.fn(),
  starts: vi.fn(),
  unlistens: [] as ReturnType<typeof vi.fn>[],
  interactiveTools: new Set<string>(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, listener: HostListener) => {
    host.listeners.push(listener);
    const unlisten = vi.fn();
    host.unlistens.push(unlisten);
    return unlisten;
  }),
}));

vi.mock("../ipc/bindings", () => ({
  commands: {
    agentHostSend: host.sends,
    agentHostStart: host.starts,
  },
}));

vi.mock("./clientToolDispatcher", () => ({
  INTERACTIVE_CLIENT_TOOLS: host.interactiveTools,
  runLauncherClientTool: vi.fn(async () => null),
  unwrap: async (promise: Promise<{ status: "ok"; data: unknown } | { status: "error"; error: string }>) => {
    const result = await promise;
    if (result.status === "error") throw new Error(result.error);
    return result.data;
  },
}));

function emit(listener: HostListener, message: LocalRuntimeInboundMessage): void {
  listener({ payload: { line: JSON.stringify(message) } });
}

function outboundTurns(): Array<Extract<LocalRuntimeOutboundMessage, { type: "turn" }>> {
  return host.sends.mock.calls
    .map(([line]) => JSON.parse(String(line)) as LocalRuntimeOutboundMessage)
    .filter(
      (message): message is Extract<LocalRuntimeOutboundMessage, { type: "turn" }> =>
        message.type === "turn",
    );
}

function assistant(id: string, text: string): UIMessage {
  return { id, role: "assistant", parts: [{ type: "text", text }] };
}

function user(id: string, text: string): UIMessage {
  return { id, role: "user", parts: [{ type: "text", text }] };
}

function request(
  providerSession: AgentProviderSession,
  conversationId: string,
  runId: string,
  text: string,
  abortController = new AbortController(),
): AgentProviderRunRequest {
  const binding: AgentRunBinding = Object.freeze({
    conversationId,
    runId,
    providerSession,
    toolContext: Object.freeze({ root: `/${conversationId}` }),
    abortController,
  });
  return {
    binding,
    history: [user(`user-${runId}`, text)],
    onUpdate: vi.fn(),
    signal: abortController.signal,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const started = { status: "ok" as const, data: null };

describe("local runtime crash recovery", () => {
  beforeEach(() => {
    host.listeners.length = 0;
    host.unlistens.length = 0;
    host.interactiveTools.clear();
    host.sends.mockReset().mockResolvedValue({ status: "ok", data: null });
    host.starts.mockReset().mockResolvedValue(started);
  });

  afterEach(() => {
    for (const listener of host.listeners) emit(listener, { type: "host_exit" });
  });

  it("ends the crashed turn and starts one fresh host for isolated next turns", async () => {
    let providerSequence = 0;
    let runSequence = 0;
    let messageSequence = 0;
    let coordinator!: AgentRunCoordinator;
    const waitForInteractiveTool = (
      binding: AgentRunBinding,
      name: string,
      toolCallId: string,
    ) => coordinator.waitForInteractiveTool(binding, name, toolCallId);
    coordinator = new AgentRunCoordinator({
      createProviderSession: ({ conversationId }) =>
        createLocalRuntimeAgent(
          "build",
          { waitForInteractiveTool },
          `provider-${conversationId}-${++providerSequence}`,
        ),
      isAutomaticTool: () => false,
      isInteractiveTool: () => false,
      runAutomaticTool: async () => null,
      onChange: () => undefined,
      makeRunId: () => `run-${++runSequence}`,
      makeMessageId: () => `message-${++messageSequence}`,
    });
    coordinator.openConversation("A", { messages: [], toolContext: { root: "/A" } });
    coordinator.openConversation("B", { messages: [], toolContext: { root: "/B" } });

    const crashed = coordinator.sendMessage("A", "crash this turn");
    await vi.waitFor(() => expect(outboundTurns()).toHaveLength(1));
    const oldListener = host.listeners[0];
    expect(host.starts).toHaveBeenCalledTimes(1);

    emit(oldListener, { type: "host_exit" });
    emit(oldListener, { type: "host_exit" });
    await crashed;

    expect(coordinator.getConversation("A")).toMatchObject({
      error: "local agent host exited",
      streaming: false,
    });
    expect(host.starts).toHaveBeenCalledTimes(1);
    expect(host.unlistens[0]).toHaveBeenCalledTimes(1);

    const recoveredA = coordinator.sendMessage("A", "recover A");
    const recoveredB = coordinator.sendMessage("B", "recover B");
    await vi.waitFor(() => {
      expect(host.starts).toHaveBeenCalledTimes(2);
      expect(host.listeners).toHaveLength(2);
      expect(outboundTurns()).toHaveLength(3);
    });

    const freshListener = host.listeners[1];
    const turns = outboundTurns();
    const turnA = turns.find((turn) => turn.conversationId === "A" && turn.runId !== "run-1");
    const turnB = turns.find((turn) => turn.conversationId === "B");
    expect(turnA).toBeDefined();
    expect(turnB).toBeDefined();

    emit(oldListener, {
      type: "update",
      providerSessionId: turnA!.providerSessionId,
      conversationId: turnA!.conversationId,
      runId: turnA!.runId,
      message: assistant("stale-old-host", "must be ignored"),
    });
    emit(freshListener, {
      type: "update",
      providerSessionId: turnB!.providerSessionId,
      conversationId: turnB!.conversationId,
      runId: turnB!.runId,
      message: assistant("answer-B", "only B"),
    });
    expect(coordinator.getConversation("A").messages).not.toContainEqual(
      assistant("stale-old-host", "must be ignored"),
    );
    expect(coordinator.getConversation("A").messages).not.toContainEqual(
      assistant("answer-B", "only B"),
    );

    emit(freshListener, {
      type: "update",
      providerSessionId: turnA!.providerSessionId,
      conversationId: turnA!.conversationId,
      runId: turnA!.runId,
      message: assistant("answer-A", "fresh A"),
    });
    emit(freshListener, {
      type: "done",
      providerSessionId: turnA!.providerSessionId,
      conversationId: turnA!.conversationId,
      runId: turnA!.runId,
    });
    emit(freshListener, {
      type: "done",
      providerSessionId: turnB!.providerSessionId,
      conversationId: turnB!.conversationId,
      runId: turnB!.runId,
    });
    await Promise.all([recoveredA, recoveredB]);

    expect(coordinator.getConversation("A")).toMatchObject({
      error: null,
      streaming: false,
    });
    expect(coordinator.getConversation("A").messages.at(-1)).toEqual(
      assistant("answer-A", "fresh A"),
    );
    expect(coordinator.getConversation("B").messages.at(-1)).toEqual(
      assistant("answer-B", "only B"),
    );
  });

  it("does not miss a host exit that overlaps startup", async () => {
    host.starts.mockImplementationOnce(async () => {
      for (const listener of host.listeners) emit(listener, { type: "host_exit" });
      return started;
    });

    await expect(createLocalRuntimeAgent(
      "build",
      { waitForInteractiveTool: async () => null },
      "provider-startup",
    )).rejects.toThrow("local agent host exited during startup");

    expect(host.starts).toHaveBeenCalledTimes(1);
    expect(host.sends).not.toHaveBeenCalled();

    await createLocalRuntimeAgent(
      "build",
      { waitForInteractiveTool: async () => null },
      "provider-recovered",
    );
    expect(host.starts).toHaveBeenCalledTimes(2);
  });

  it("shares a rejected start but retries only for a later explicit request", async () => {
    host.starts.mockRejectedValueOnce(new Error("host start rejected"));
    const hooks = { waitForInteractiveTool: async () => null };

    const first = createLocalRuntimeAgent("build", hooks, "provider-A");
    const overlapping = createLocalRuntimeAgent("build", hooks, "provider-B");
    await expect(Promise.all([first, overlapping])).rejects.toThrow("host start rejected");

    expect(host.starts).toHaveBeenCalledTimes(1);
    expect(host.sends).not.toHaveBeenCalled();

    await createLocalRuntimeAgent("build", hooks, "provider-C");
    expect(host.starts).toHaveBeenCalledTimes(2);
  });

  it("invalidates a dead generation after send rejection without replaying its turn", async () => {
    const agent = await createLocalRuntimeAgent(
      "build",
      { waitForInteractiveTool: async () => null },
      "provider-A",
    );
    host.sends.mockRejectedValueOnce(new Error("host pipe closed"));

    await expect(agent.run(request(agent, "A", "run-failed", "failed once")))
      .resolves.toMatchObject({ error: "local agent host exited" });
    expect(host.starts).toHaveBeenCalledTimes(1);

    const later = agent.run(request(agent, "A", "run-later", "later explicit"));
    await vi.waitFor(() => expect(host.starts).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(outboundTurns()).toHaveLength(2));

    emit(host.listeners.at(-1)!, {
      type: "done",
      providerSessionId: "provider-A",
      conversationId: "A",
      runId: "run-later",
    });
    await later;

    expect(outboundTurns().map((turn) => turn.text)).toEqual([
      "failed once",
      "later explicit",
    ]);
    expect(host.unlistens[0]).toHaveBeenCalledTimes(1);
  });

  it("settles every active wrapper when one send proves the shared host dead", async () => {
    const hooks = { waitForInteractiveTool: async () => null };
    const agentA = await createLocalRuntimeAgent("build", hooks, "provider-A");
    const agentB = await createLocalRuntimeAgent("build", hooks, "provider-B");
    const runningA = agentA.run(request(agentA, "A", "run-A", "alpha"));
    await vi.waitFor(() => expect(outboundTurns()).toHaveLength(1));
    host.sends.mockRejectedValueOnce(new Error("shared host pipe closed"));

    let resultA: Awaited<ReturnType<AgentProviderSession["run"]>> | undefined;
    void runningA.then((result) => { resultA = result; });
    const resultB = await agentB.run(request(agentB, "B", "run-B", "bravo"));
    await Promise.resolve();
    await Promise.resolve();

    expect(resultA).toMatchObject({ error: "local agent host exited" });
    expect(resultB).toMatchObject({ error: "local agent host exited" });
    expect(host.starts).toHaveBeenCalledTimes(1);
  });

  it("settles an aborted turn while a shared host restart is still pending", async () => {
    const agent = await createLocalRuntimeAgent(
      "build",
      { waitForInteractiveTool: async () => null },
      "provider-A",
    );
    emit(host.listeners[0], { type: "host_exit" });
    const restart = deferred<typeof started>();
    host.starts.mockImplementationOnce(() => restart.promise);
    const abortController = new AbortController();
    const runRequest = request(agent, "A", "run-aborted", "cancel restart", abortController);
    const running = agent.run(runRequest);
    await vi.waitFor(() => expect(host.starts).toHaveBeenCalledTimes(2));

    let settled = false;
    void running.then(() => { settled = true; });
    abortController.abort();
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    try {
      expect(settled).toBe(true);
      expect(host.sends).not.toHaveBeenCalled();
    } finally {
      restart.resolve(started);
      await vi.waitFor(() => expect(host.listeners).toHaveLength(2));
      emit(host.listeners[1], { type: "host_exit" });
      await running;
    }
  });

  it("routes interactive calls through the wrapper that owns the run binding", async () => {
    host.interactiveTools.add("ask_user_question");
    const waitA = vi.fn(async () => ({ answer: "A" }));
    const waitB = vi.fn(async () => ({ answer: "B" }));
    const agentA = await createLocalRuntimeAgent(
      "build",
      { waitForInteractiveTool: waitA },
      "provider-A",
    );
    const agentB = await createLocalRuntimeAgent(
      "build",
      { waitForInteractiveTool: waitB },
      "provider-B",
    );
    const requestA = request(agentA, "A", "run-A", "alpha");
    const requestB = request(agentB, "B", "run-B", "bravo");
    const runningA = agentA.run(requestA);
    const runningB = agentB.run(requestB);
    await vi.waitFor(() => expect(outboundTurns()).toHaveLength(2));

    emit(host.listeners[0], {
      type: "tool_call",
      providerSessionId: "provider-B",
      conversationId: "B",
      runId: "run-B",
      toolCallId: "question-B",
      name: "ask_user_question",
      args: { question: "B?" },
    });

    try {
      expect(waitB).toHaveBeenCalledWith(
        requestB.binding,
        "ask_user_question",
        "question-B",
      );
      expect(waitA).not.toHaveBeenCalled();
    } finally {
      emit(host.listeners[0], {
        type: "done",
        providerSessionId: "provider-A",
        conversationId: "A",
        runId: "run-A",
      });
      emit(host.listeners[0], {
        type: "done",
        providerSessionId: "provider-B",
        conversationId: "B",
        runId: "run-B",
      });
      await Promise.all([runningA, runningB]);
    }
  });
});
