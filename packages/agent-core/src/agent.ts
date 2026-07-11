// The OpenRouter engine — the API-key-based entrypoint of the brain.
//
// The loop is the AI SDK's standard `ToolLoopAgent` (the same `Agent` (agent-v1)
// interface the local-runtime engine's `HarnessAgent` implements — the two
// engines differ only in who owns the model loop, not in shape). One call runs
// ONE turn: the agent streams text/reasoning/tool requests until a final answer,
// client-side tool pause, or the step cap. We consume its
// native UI-message stream: `agent.stream().toUIMessageStream()` →
// `readUIMessageStream()` accumulates a growing `UIMessage` (text / reasoning /
// tool parts with an input-streaming → available → output state machine), which
// the caller renders directly. History is kept as `UIMessage[]` (single source
// for render AND model, via `convertToModelMessages` — done here, not via the
// `createAgentUIStream` sugar, because we need `ignoreIncompleteToolCalls`: an
// interrupted turn can leave a dangling tool call in history).
//
// All tools are native client-side tools (no `execute`): the turn pauses with
// tool parts in `input-available` (no output). The launcher client runs the tool
// through Rust IPC or UI, sets the part to `output-available`, and calls `run`
// again to continue the SAME conversation.

import { ToolLoopAgent, stepCountIs, convertToModelMessages, readUIMessageStream, type UIMessage } from "ai";
import { createOpenRouter } from "@openrouter/ai-sdk-provider";

import { promptForMode } from "./prompt";
import { buildTools } from "./tools";
import type { AgentLlmSettings, AgentMode } from "./types";

/** Max tool round-trips per turn. */
const MAX_STEPS = 16;
/** Low temperature — this is an orchestrator, not a poet. */
const TEMPERATURE = 0.3;
/** Per-turn output token budget. */
const MAX_OUTPUT_TOKENS = 2048;

/** Result of a turn: the full updated `UIMessage[]` history + an optional error. */
export interface TurnResult {
  messages: UIMessage[];
  error?: string;
}

export interface ModpackAgent {
  /**
   * Stream one assistant turn from the given `UIMessage[]` history, calling
   * `onUpdate` on every growth of the assistant message, and resolving with the
   * updated history (`[...history, assistant]`). Never rejects — a model/tool
   * failure surfaces as `TurnResult.error` and whatever streamed so far is kept.
   *
   * The caller owns history mutations: to send a message, append a user
   * `UIMessage` first; to resume after any client-side tool, set that tool part
   * to `output-available` in the last assistant message.
   * Either way, just pass the full history — a completed tool result is fed back to
   * the model via `convertToModelMessages`; a pending one pauses the turn.
   */
  run(
    history: UIMessage[],
    onUpdate: (assistant: UIMessage) => void,
    signal?: AbortSignal,
  ): Promise<TurnResult>;
}

export interface AgentRuntimeOptions {
  mode?: AgentMode;
}

interface UIStreamResult<TStream> {
  toUIMessageStream(options?: { sendReasoning?: boolean }): TStream;
}

interface RunUiMessageTurnOptions<TStream, TMessage> {
  history: UIMessage[];
  onUpdate: (assistant: UIMessage) => void;
  signal?: AbortSignal;
  start: () => Promise<UIStreamResult<TStream>>;
  readUIMessageStream: (options: {
    stream: TStream;
    onError: (error: unknown) => void;
  }) => AsyncIterable<TMessage>;
  mapMessage: (message: TMessage) => UIMessage;
}

/**
 * Shared turn runner for all engines. The engine decides how to start the LLM
 * stream; this layer owns UIMessage accumulation, update callbacks, abort
 * semantics, and `TurnResult` shape.
 */
export async function runUiMessageTurn<TStream, TMessage>({
  history,
  onUpdate,
  signal,
  start,
  readUIMessageStream: read,
  mapMessage,
}: RunUiMessageTurnOptions<TStream, TMessage>): Promise<TurnResult> {
  let error: string | undefined;
  let assistant: UIMessage | undefined;
  try {
    const result = await start();
    const uiStream = result.toUIMessageStream({ sendReasoning: true });
    for await (const msg of read({
      stream: uiStream,
      onError: (e) => {
        error = errText(e);
      },
    })) {
      assistant = mapMessage(msg);
      onUpdate(assistant);
    }
  } catch (e) {
    // A user interrupt (AbortSignal) is a clean stop, not a failure: keep the
    // partial assistant that streamed so far and surface no error.
    if (!isAbort(e, signal)) error = errText(e);
  }
  return { messages: assistant ? [...history, assistant] : history, error };
}

/**
 * Create a modpack agent bound to an LLM endpoint.
 * The provider is an OpenAI-compatible client over `settings.baseUrl`.
 */
export function createModpackAgent(
  settings: AgentLlmSettings,
  options: AgentRuntimeOptions = {},
): ModpackAgent {
  const mode = options.mode ?? "modpack";
  const provider = createOpenRouter({ apiKey: settings.apiKey, baseURL: settings.baseUrl });
  const toolSet = buildTools(mode);
  const agent = new ToolLoopAgent({
    model: provider.chat(settings.model),
    instructions: promptForMode(mode),
    tools: toolSet,
    temperature: TEMPERATURE,
    maxOutputTokens: MAX_OUTPUT_TOKENS,
    stopWhen: stepCountIs(MAX_STEPS),
  });

  function run(
    history: UIMessage[],
    onUpdate: (assistant: UIMessage) => void,
    signal?: AbortSignal,
  ): Promise<TurnResult> {
    return runUiMessageTurn({
      history,
      onUpdate,
      signal,
      start: async () => {
        const modelMessages = await convertToModelMessages(history, {
          tools: toolSet,
          ignoreIncompleteToolCalls: true,
        });
        return agent.stream({ prompt: modelMessages, abortSignal: signal });
      },
      readUIMessageStream,
      mapMessage: (msg) => msg as UIMessage,
    });
  }

  return { run };
}

function errText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

/** True when the thrown error is the caller's own abort (interrupt), not a real failure. */
function isAbort(e: unknown, signal?: AbortSignal): boolean {
  return signal?.aborted === true || (e instanceof Error && e.name === "AbortError");
}
