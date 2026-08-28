import { beforeEach, describe, expect, it, vi } from "vitest";
import type { UIMessage } from "ai";
import { createLocalRuntimeAgent } from "./localRuntimeAdapter";
import {
  AgentRunCoordinator,
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
  INTERACTIVE_CLIENT_TOOLS: new Set<string>(),
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

describe("local runtime crash recovery", () => {
  beforeEach(() => {
    host.listeners.length = 0;
    host.unlistens.length = 0;
    host.sends.mockReset().mockResolvedValue({ status: "ok", data: null });
    host.starts.mockReset().mockResolvedValue({ status: "ok", data: null });
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
});
