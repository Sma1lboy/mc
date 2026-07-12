import type { UIMessage } from "ai";
import type { AgentMode } from "@kobemc/agent-core";
import type { AgentProviderRunRequest, AgentRunBinding } from "./runCoordinator";

export type LocalRuntimeOutboundMessage =
  | {
      type: "turn";
      conversationId: string;
      runId: string;
      text: string;
      mode: AgentMode;
    }
  | { type: "abort"; conversationId: string; runId: string }
  | {
      type: "tool_result";
      conversationId: string;
      runId: string;
      toolCallId: string;
      ok: true;
      result: unknown;
    }
  | {
      type: "tool_result";
      conversationId: string;
      runId: string;
      toolCallId: string;
      ok: false;
      error: string;
    };

export type LocalRuntimeInboundMessage =
  | { type: "update"; conversationId: string; runId: string; message: UIMessage }
  | {
      type: "tool_call";
      conversationId: string;
      runId: string;
      toolCallId: string;
      name: string;
      args: unknown;
    }
  | { type: "done"; conversationId: string; runId: string; error?: string }
  | { type: "host_exit" };

interface ActiveTurn {
  request: AgentProviderRunRequest;
  assistant?: UIMessage;
  finish: (error?: string) => void;
  removeAbortListener: () => void;
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
}

export function createLocalRuntimeProtocol(options: LocalRuntimeProtocolOptions) {
  const active = new Map<string, ActiveTurn>();

  function key(conversationId: string, runId: string): string {
    return `${conversationId}\u0000${runId}`;
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
  ): Promise<{ messages: UIMessage[]; error?: string }> {
    const { conversationId, runId } = request.binding;
    const activeKey = key(conversationId, runId);
    if (active.has(activeKey)) {
      return Promise.resolve({ messages: request.history, error: "run already active" });
    }
    return new Promise((resolve) => {
      const abort = () => {
        void options.send({ type: "abort", conversationId, runId });
      };
      request.signal.addEventListener("abort", abort, { once: true });
      const turn: ActiveTurn = {
        request,
        finish: (error) => {
          if (!active.delete(activeKey)) return;
          turn.removeAbortListener();
          resolve({
            messages: turn.assistant ? [...request.history, turn.assistant] : request.history,
            error: request.signal.aborted ? undefined : error,
          });
        },
        removeAbortListener: () => request.signal.removeEventListener("abort", abort),
      };
      active.set(activeKey, turn);
      void Promise.resolve(
        options.send({
          type: "turn",
          conversationId,
          runId,
          text: newestUserText(request.history),
          mode,
        }),
      ).catch((error) => turn.finish(error instanceof Error ? error.message : String(error)));
    });
  }

  function handleToolCall(message: Extract<LocalRuntimeInboundMessage, { type: "tool_call" }>) {
    const turn = active.get(key(message.conversationId, message.runId));
    if (!turn) return;
    const execution = options.isInteractiveTool(message.name)
      ? options.waitForInteractiveTool(turn.request.binding, message.name, message.toolCallId)
      : options.runAutomaticTool(
          message.name,
          message.args,
          turn.request.binding.toolContext,
        );
    void execution.then(
      (result) =>
        options.send({
          type: "tool_result",
          conversationId: message.conversationId,
          runId: message.runId,
          toolCallId: message.toolCallId,
          ok: true,
          result,
        }),
      (error) =>
        options.send({
          type: "tool_result",
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
      for (const turn of active.values()) turn.finish("local agent host exited");
      return;
    }
    const turn = active.get(key(message.conversationId, message.runId));
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
        turn.finish(message.error);
        break;
    }
  }

  return { run, handle };
}
