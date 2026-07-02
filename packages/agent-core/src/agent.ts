// The streaming tool-use loop — the public entrypoint of the brain.
//
// Mirrors Rust `mc_core::agent::chat::run::run_chat_turn`: one call runs ONE
// user turn, letting the model stream text/reasoning and call the deterministic
// tools (the SDK auto-dispatches them and feeds results back) until it produces a
// final answer or hits the step cap. Every step is forwarded as an
// `AgentStreamEvent`; the updated transcript is returned to seed the next turn.

import { streamText, stepCountIs, type ModelMessage } from "ai";
import { createOpenRouter } from "@openrouter/ai-sdk-provider";

import { CHAT_AGENT_SYSTEM_PROMPT } from "./prompt";
import { buildTools } from "./tools";
import type { AgentLlmSettings, AgentStreamEvent, ToolExecutor } from "./types";

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

  return {
    async runTurn(history, userMessage, onEvent) {
      const input: ModelMessage[] = [...history, { role: "user", content: userMessage }];
      let reply = "";
      try {
        const result = streamText({
          model,
          system: CHAT_AGENT_SYSTEM_PROMPT,
          messages: input,
          tools: toolSet,
          temperature: TEMPERATURE,
          maxOutputTokens: MAX_OUTPUT_TOKENS,
          stopWhen: stepCountIs(MAX_STEPS),
        });

        for await (const part of result.fullStream) {
          switch (part.type) {
            case "text-delta":
              reply += part.text;
              onEvent({ type: "text_delta", delta: part.text });
              break;
            case "reasoning-delta":
              if (part.text) onEvent({ type: "reasoning", delta: part.text });
              break;
            case "tool-call":
              onEvent({ type: "tool_call", name: part.toolName, args: part.input });
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
            // text-start/-end, reasoning-start/-end, tool-input-*, start/finish(-step),
            // source, file, raw, abort: internal progress we don't surface.
          }
        }

        // `.response` resolves once the stream is drained, carrying the assistant
        // (and tool) messages generated this turn — the transcript delta to keep.
        const response = await result.response;
        onEvent({ type: "done" });
        return { history: [...input, ...response.messages], reply };
      } catch (e) {
        onEvent({ type: "error", message: errText(e) });
        onEvent({ type: "done" });
        return { history: input, reply };
      }
    },
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
