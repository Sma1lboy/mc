// The streaming tool-use loop — the public entrypoint of the brain.
//
// Mirrors Rust `the retired Rust chat brain`: one call runs ONE
// user turn, letting the model stream text/reasoning and call the deterministic
// tools (the SDK auto-dispatches them and feeds results back) until it produces a
// final answer or hits the step cap. Every step is forwarded as an
// `AgentStreamEvent`; the updated transcript is returned to seed the next turn.

import { streamText, stepCountIs, parsePartialJson, type ModelMessage } from "ai";
import { createOpenRouter } from "@openrouter/ai-sdk-provider";

import { CHAT_AGENT_SYSTEM_PROMPT } from "./prompt";
import { buildTools, ASK_USER_TOOL } from "./tools";
import type { AgentLlmSettings, AgentStreamEvent, AskUserOption, ToolExecutor } from "./types";

/** Max tool round-trips per turn (matches Rust MAX_TOOL_TURNS). */
const MAX_STEPS = 16;
/** Low temperature — this is an orchestrator, not a poet (matches Rust). */
const TEMPERATURE = 0.3;
/** Per-turn output token budget (matches Rust CHAT_MAX_TOKENS). */
const MAX_OUTPUT_TOKENS = 2048;
/** Cap on a tool-result summary emitted to the sink (chars, ~Rust's 240). */
const TOOL_SUMMARY_MAX_CHARS = 200;

export interface ModpackAgent {
  /**
   * Run one streaming turn. Appends `userMessage` to `history`, streams events
   * through `onEvent`, and resolves with the concatenated `reply` plus the
   * updated `history` (input + the SDK's response messages) to feed the next
   * turn. Never rejects for a model/tool failure — those surface as an `error`
   * event and the input history is returned unchanged.
   */
  runTurn(
    history: ModelMessage[],
    userMessage: string,
    onEvent: (event: AgentStreamEvent) => void,
  ): Promise<{ history: ModelMessage[]; reply: string }>;

  /**
   * Resume a turn that paused on a client-side tool (`ask_user_question`): feed
   * the user's pick back as that tool call's result and continue the SAME turn.
   * `history` must be the history returned by the paused `runTurn` (it ends with
   * the assistant tool-call message). `output` is the tool result the model reads.
   */
  resumeTurn(
    history: ModelMessage[],
    toolCallId: string,
    output: unknown,
    onEvent: (event: AgentStreamEvent) => void,
  ): Promise<{ history: ModelMessage[]; reply: string }>;
}

/**
 * Create a modpack agent bound to an LLM endpoint and a host tool backend.
 * The provider is an OpenAI-compatible client over `settings.baseUrl`, so this
 * runs against OpenRouter today and any self-hosted compatible endpoint later.
 */
export function createModpackAgent(settings: AgentLlmSettings, tools: ToolExecutor): ModpackAgent {
  const provider = createOpenRouter({ apiKey: settings.apiKey, baseURL: settings.baseUrl });
  const model = provider.chat(settings.model);
  const toolSet = buildTools(tools);

  // One streaming pass over `messages`: forwards events, returns the response
  // messages (transcript delta) + concatenated reply. Shared by runTurn/resumeTurn.
  // Never throws — model/tool failures surface as an `error` event; on failure the
  // response delta is empty so the caller keeps the prior history.
  async function streamPass(
    messages: ModelMessage[],
    onEvent: (event: AgentStreamEvent) => void,
  ): Promise<{ delta: ModelMessage[]; reply: string }> {
    let reply = "";
    try {
      const result = streamText({
        model,
        system: CHAT_AGENT_SYSTEM_PROMPT,
        messages,
        tools: toolSet,
        temperature: TEMPERATURE,
        maxOutputTokens: MAX_OUTPUT_TOKENS,
        stopWhen: stepCountIs(MAX_STEPS),
      });

      // Accumulate ask_user's argument JSON as it streams (keyed by tool-call id),
      // so the UI can render the chip frame immediately on the header and fill in
      // the question/options progressively via partial-JSON parsing — instead of a
      // blank gap until the whole call finishes. Only ask_user needs this.
      const askArgs = new Map<string, string>();

      for await (const part of result.fullStream) {
        switch (part.type) {
          case "text-delta":
            reply += part.text;
            onEvent({ type: "text_delta", delta: part.text });
            break;
          case "reasoning-delta":
            if (part.text) onEvent({ type: "reasoning", delta: part.text });
            break;
          case "tool-input-start":
            // Header arrived (id + toolName) before the args finished streaming.
            // For ask_user, emit an empty chip now so the UI shows the frame at once.
            if (part.toolName === ASK_USER_TOOL) {
              askArgs.set(part.id, "");
              onEvent(askUserEvent(part.id, {}));
            }
            break;
          case "tool-input-delta":
            // ask_user only: append the raw arg-JSON delta, parse what's parseable so
            // far, and re-emit (the UI upserts by tool_call_id → chips fill in live).
            if (askArgs.has(part.id)) {
              const text = (askArgs.get(part.id) ?? "") + part.delta;
              askArgs.set(part.id, text);
              const { value } = await parsePartialJson(text);
              onEvent(askUserEvent(part.id, value ?? {}));
            }
            break;
          case "tool-call":
            // Final, validated args. ask_user renders as interactive chips; the SDK
            // pauses the turn on it (no executor) until we resume with a result.
            if (part.toolName === ASK_USER_TOOL) onEvent(askUserEvent(part.toolCallId, part.input));
            else onEvent({ type: "tool_call", name: part.toolName, args: part.input });
            break;
          case "tool-result":
            onEvent({ type: "tool_result", name: part.toolName, summary: summarize(part.output) });
            break;
          case "tool-error":
            onEvent({ type: "tool_result", name: part.toolName, summary: `error: ${errText(part.error)}` });
            break;
          case "error":
            onEvent({ type: "error", message: errText(part.error) });
            break;
          // text-start/-end, reasoning-start/-end, tool-input-end, start/finish(-step),
          // source, file, raw, abort: internal progress we don't surface.
        }
      }

      // `.response` resolves once the stream is drained, carrying the assistant
      // (and tool) messages generated this pass — the transcript delta to keep.
      const response = await result.response;
      onEvent({ type: "done" });
      return { delta: response.messages, reply };
    } catch (e) {
      onEvent({ type: "error", message: errText(e) });
      onEvent({ type: "done" });
      return { delta: [], reply };
    }
  }

  return {
    async runTurn(history, userMessage, onEvent) {
      const input: ModelMessage[] = [...history, { role: "user", content: userMessage }];
      const { delta, reply } = await streamPass(input, onEvent);
      return { history: [...input, ...delta], reply };
    },

    async resumeTurn(history, toolCallId, output, onEvent) {
      // Feed the user's pick back as the paused tool call's result, then continue.
      const toolMsg: ModelMessage = {
        role: "tool",
        content: [{ type: "tool-result", toolCallId, toolName: ASK_USER_TOOL, output: { type: "json", value: output as never } }],
      };
      const input: ModelMessage[] = [...history, toolMsg];
      const { delta, reply } = await streamPass(input, onEvent);
      return { history: [...input, ...delta], reply };
    },
  };
}

/** Shape the validated `ask_user_question` tool input into an `ask_user` event. */
function askUserEvent(toolCallId: string, input: unknown): AgentStreamEvent {
  const raw = (input ?? {}) as {
    question?: unknown;
    options?: unknown;
    multi_select?: unknown;
  };
  const options: AskUserOption[] = Array.isArray(raw.options)
    ? raw.options.flatMap((o) => {
        const opt = (o ?? {}) as { label?: unknown; id?: unknown; description?: unknown };
        const label = typeof opt.label === "string" ? opt.label : "";
        if (!label) return [];
        return [
          {
            label,
            ...(typeof opt.id === "string" ? { id: opt.id } : {}),
            ...(typeof opt.description === "string" ? { description: opt.description } : {}),
          },
        ];
      })
    : [];
  return {
    type: "ask_user",
    tool_call_id: toolCallId,
    question: typeof raw.question === "string" ? raw.question : "",
    options,
    multi_select: raw.multi_select === true,
  };
}

/** Flatten a tool output into a short, single-line JSON summary for the sink. */
function summarize(output: unknown): string {
  let text: string;
  try {
    text = typeof output === "string" ? output : JSON.stringify(output);
  } catch {
    text = String(output);
  }
  text = (text ?? "").replace(/\s+/g, " ").trim();
  if (!text) return "(no textual result)";
  return text.length > TOOL_SUMMARY_MAX_CHARS ? `${text.slice(0, TOOL_SUMMARY_MAX_CHARS)}…` : text;
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
