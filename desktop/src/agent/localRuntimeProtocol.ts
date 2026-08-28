import type { UIMessage } from "ai";
import type { AgentMode } from "@kobemc/agent-core";
import {
  DEFAULT_CANCELLATION_GRACE_MS,
  type AgentProviderRunRequest,
  type AgentRunBinding,
} from "./runCoordinator";

export type LocalRuntimeOutboundMessage =
  | {
      type: "turn";
      providerSessionId: string;
      conversationId: string;
      runId: string;
      text: string;
      mode: AgentMode;
    }
  | { type: "abort"; providerSessionId: string; conversationId: string; runId: string }
  | {
      type: "tool_result";
      providerSessionId: string;
      conversationId: string;
      runId: string;
      toolCallId: string;
      ok: true;
      result: unknown;
    }
  | {
      type: "tool_result";
      providerSessionId: string;
      conversationId: string;
      runId: string;
      toolCallId: string;
      ok: false;
      error: string;
    };

export type LocalRuntimeInboundMessage =
  | { type: "update"; providerSessionId: string; conversationId: string; runId: string; message: UIMessage }
  | {
      type: "tool_call";
      providerSessionId: string;
      conversationId: string;
      runId: string;
      toolCallId: string;
      name: string;
      args: unknown;
    }
  | {
      type: "done";
      providerSessionId: string;
      conversationId: string;
      runId: string;
      error?: string;
      promptVersion?: string;
    }
  | { type: "host_exit" };

interface ActiveTurn {
  request: AgentProviderRunRequest;
  assistant?: UIMessage;
  finish: (done: { error?: string; promptVersion?: string }) => void;
  removeAbortListener: () => void;
  cancellationTimer?: ReturnType<typeof setTimeout>;
  toolCallIds: Set<string>;
}

interface LocalRuntimeProtocolOptions {
  send: (message: LocalRuntimeOutboundMessage) => void | Promise<void>;
  isInteractiveTool: (name: string) => boolean;
  runAutomaticTool: (name: string, input: unknown, toolContext: unknown) => Promise<unknown>;
  waitForInteractiveTool: (
    binding: AgentRunBinding,
    name: string,
    toolCallId: string,
  ) => Promise<unknown>;
  cancellationGraceMs?: number;
}

export function createLocalRuntimeProtocol(options: LocalRuntimeProtocolOptions) {
  const active = new Map<string, ActiveTurn>();

  function key(providerSessionId: string, conversationId: string, runId: string): string {
    return `${providerSessionId}\u0000${conversationId}\u0000${runId}`;
  }

  function newestUserText(history: UIMessage[]): string {
    const lastUser = [...history].reverse().find((message) => message.role === "user");
    return (lastUser?.parts ?? [])
      .map((part) => (part.type === "text" ? part.text : ""))
      .join("");
  }

  function run(
    request: AgentProviderRunRequest,
    mode: AgentMode,
    providerSessionId: string,
  ): Promise<{ messages: UIMessage[]; error?: string; promptVersion?: string }> {
    const { conversationId, runId } = request.binding;
    const activeKey = key(providerSessionId, conversationId, runId);
    if (active.has(activeKey)) {
      return Promise.resolve({ messages: request.history, error: "run already active" });
    }
    return new Promise((resolve) => {
      let turn: ActiveTurn;
      let abortSent = false;
      const abort = () => {
        if (abortSent) return;
        abortSent = true;
        turn.cancellationTimer = setTimeout(
          () => turn.finish({}),
          Math.max(0, options.cancellationGraceMs ?? DEFAULT_CANCELLATION_GRACE_MS),
        );
        sendSafely(
          { type: "abort", providerSessionId, conversationId, runId },
          () => turn.finish({}),
        );
      };
      request.signal.addEventListener("abort", abort, { once: true });
      turn = {
        request,
        finish: (done) => {
          if (active.get(activeKey) !== turn) return;
          active.delete(activeKey);
          if (turn.cancellationTimer) clearTimeout(turn.cancellationTimer);
          turn.removeAbortListener();
          resolve({
            messages: turn.assistant ? [...request.history, turn.assistant] : request.history,
            error: request.signal.aborted ? undefined : done.error,
            ...(done.promptVersion ? { promptVersion: done.promptVersion } : {}),
          });
        },
        removeAbortListener: () => request.signal.removeEventListener("abort", abort),
        toolCallIds: new Set(),
      };
      active.set(activeKey, turn);
      if (request.signal.aborted) {
        abort();
        return;
      }
      sendSafely(
        {
          type: "turn",
          providerSessionId,
          conversationId,
          runId,
          text: newestUserText(request.history),
          mode,
        },
        (error) =>
          turn.finish({ error: error instanceof Error ? error.message : String(error) }),
      );
    });
  }

  function sendSafely(
    message: LocalRuntimeOutboundMessage,
    onError: (error: unknown) => void,
  ): void {
    try {
      void Promise.resolve(options.send(message)).catch(onError);
    } catch (error) {
      onError(error);
    }
  }

  function handleToolCall(message: Extract<LocalRuntimeInboundMessage, { type: "tool_call" }>) {
    const activeKey = key(message.providerSessionId, message.conversationId, message.runId);
    const turn = active.get(activeKey);
    if (!turn || turn.toolCallIds.has(message.toolCallId)) return;
    turn.toolCallIds.add(message.toolCallId);
    let execution: Promise<unknown>;
    try {
      execution = options.isInteractiveTool(message.name)
        ? options.waitForInteractiveTool(turn.request.binding, message.name, message.toolCallId)
        : options.runAutomaticTool(
            message.name,
            message.args,
            turn.request.binding.toolContext,
          );
    } catch (error) {
      execution = Promise.reject(error);
    }
    const sendResult = (outbound: LocalRuntimeOutboundMessage) => {
      if (active.get(activeKey) !== turn || turn.request.signal.aborted) return;
      sendSafely(outbound, (error) =>
        turn.finish({ error: error instanceof Error ? error.message : String(error) }),
      );
    };
    void execution.then(
      (result) =>
        sendResult({
          type: "tool_result",
          providerSessionId: message.providerSessionId,
          conversationId: message.conversationId,
          runId: message.runId,
          toolCallId: message.toolCallId,
          ok: true,
          result,
        }),
      (error) =>
        sendResult({
          type: "tool_result",
          providerSessionId: message.providerSessionId,
          conversationId: message.conversationId,
          runId: message.runId,
          toolCallId: message.toolCallId,
          ok: false,
          error: error instanceof Error ? error.message : String(error),
        }),
    );
  }

  function handle(message: LocalRuntimeInboundMessage): void {
    if (message.type === "host_exit") {
      for (const turn of active.values()) turn.finish({ error: "local agent host exited" });
      return;
    }
    const turn = active.get(key(message.providerSessionId, message.conversationId, message.runId));
    if (!turn) return;
    switch (message.type) {
      case "update":
        turn.assistant = message.message;
        turn.request.onUpdate(message.message);
        break;
      case "tool_call":
        handleToolCall(message);
        break;
      case "done":
        turn.finish({ error: message.error, promptVersion: message.promptVersion });
        break;
    }
  }

  return { run, handle };
}
