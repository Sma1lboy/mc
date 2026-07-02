// The streaming tool-use loop — the public entrypoint of the brain.
//
// One call runs ONE turn: the model streams text/reasoning and calls the
// deterministic tools (the SDK auto-dispatches them) until it produces a final
// answer or hits the step cap. We use the AI SDK's native UI-message stream:
// `streamText().toUIMessageStream()` → `readUIMessageStream()` accumulates a growing
// `UIMessage` (text / reasoning / tool parts with an input-streaming → available →
// output state machine), which the caller renders directly. History is kept as
// `UIMessage[]` (single source for render AND model, via `convertToModelMessages`).
//
// `ask_user_question` is a native client-side tool (no executor): the turn pauses
// with its tool part in `input-available` (no output). The caller collects the
// user's pick, sets that part to `output-available`, and calls `resumeTurn` to
// continue the SAME conversation.

import {
  streamText,
  stepCountIs,
  convertToModelMessages,
  readUIMessageStream,
  type UIMessage,
} from "ai";
import { createOpenRouter } from "@openrouter/ai-sdk-provider";

import { CHAT_AGENT_SYSTEM_PROMPT } from "./prompt";
import { buildTools } from "./tools";
import type { AgentLlmSettings, ToolExecutor } from "./types";

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
   * `UIMessage` first; to resume after the `ask_user_question` client-side tool,
   * set that tool part to `output-available` (the user's pick) in the last message.
   * Either way, just pass the full history — a completed tool result is fed back to
   * the model via `convertToModelMessages`; a pending one pauses the turn.
   */
  run(history: UIMessage[], onUpdate: (assistant: UIMessage) => void): Promise<TurnResult>;
}

/**
 * Create a modpack agent bound to an LLM endpoint and a host tool backend.
 * The provider is an OpenAI-compatible client over `settings.baseUrl`.
 */
export function createModpackAgent(settings: AgentLlmSettings, tools: ToolExecutor): ModpackAgent {
  const provider = createOpenRouter({ apiKey: settings.apiKey, baseURL: settings.baseUrl });
  const model = provider.chat(settings.model);
  const toolSet = buildTools(tools);

  // Stream one assistant turn from the given UI history. Returns the updated
  // history (+ the streamed assistant) and any error. Never throws.
  async function stream(
    uiHistory: UIMessage[],
    onUpdate: (assistant: UIMessage) => void,
  ): Promise<TurnResult> {
    let error: string | undefined;
    let assistant: UIMessage | undefined;
    try {
      const modelMessages = await convertToModelMessages(uiHistory, {
        tools: toolSet,
        ignoreIncompleteToolCalls: true,
      });
      const result = streamText({
        model,
        system: CHAT_AGENT_SYSTEM_PROMPT,
        messages: modelMessages,
        tools: toolSet,
        temperature: TEMPERATURE,
        maxOutputTokens: MAX_OUTPUT_TOKENS,
        stopWhen: stepCountIs(MAX_STEPS),
      });
      const uiStream = result.toUIMessageStream({ sendReasoning: true });
      for await (const msg of readUIMessageStream({
        stream: uiStream,
        onError: (e) => {
          error = errText(e);
        },
      })) {
        assistant = msg;
        onUpdate(msg);
      }
    } catch (e) {
      error = errText(e);
    }
    return { messages: assistant ? [...uiHistory, assistant] : uiHistory, error };
  }

  return { run: stream };
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
