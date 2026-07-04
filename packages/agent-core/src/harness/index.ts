// Local-runtime engine (Node-only): the same modpack brain, but the LLM loop is
// owned by the locally-installed Claude Code runtime instead of an API key.
//
// AI SDK HarnessAgent (ai@7) drives the runtime through a bridge; our
// `createLocalSandbox` makes that bridge run ON THIS MACHINE as the current
// user, so the runtime reuses the local `claude` subscription login — zero
// API key. Our deterministic tool schemas stay in agent-core; execution is
// bridged back to the launcher client, which supplies results through Rust IPC
// or UI. Builtin coding tools (bash/read/write/…) are denied via
// constructor-level `activeTools`.
//
// This module is imported ONLY by Node hosts (mc-agent CLI, a future desktop
// sidecar / mc-server) through the "./harness" export — never by the webview
// bundle, which cannot spawn processes.
//
// The harness runtime cannot directly call launcher IPC, so Node hosts may
// attach bridge handlers to tools. Those handlers are not tool implementations;
// they just round-trip the call to the launcher client and await its output.
//
// One live runtime session per agent; `dispose()` must be called or the
// bridge child process keeps the host alive.

import { HarnessAgent } from "@ai-sdk/harness/agent";
import { createClaudeCode } from "@ai-sdk/harness-claude-code";
// The harness stack runs on ai@7 (nested dep); readUIMessageStream must match
// its UIMessageChunk protocol, so it comes from the "ai-v7" alias — NOT from
// the ai@6 the OpenRouter engine uses. Unify when the repo moves to ai@7.
import { readUIMessageStream } from "ai-v7";
import type { UIMessage } from "ai";

import { CHAT_AGENT_SYSTEM_PROMPT } from "../prompt";
import { buildTools, ASK_USER_TOOL, SHOW_MODPACK_TOOL } from "../tools";
import type { ClientToolHandlers } from "../types";
import { runUiMessageTurn, type ModpackAgent, type TurnResult } from "../agent";
import { createLocalSandbox } from "./local-sandbox";

export { createLocalSandbox } from "./local-sandbox";

const TEXT_FALLBACK_NOTE = `

## Local-runtime mode override
The interactive tools \`ask_user_question\` and \`show_modpack\` are NOT available in this session — never call them. Instead:
- To ask the user a choice, ask in plain text with a short numbered list of options.
- To present the final pack, describe it in concise markdown (title, mc version, loader, and the built file's output_path if you ran build_modpack) and tell the user to install it from the launcher.`;

/** The two tools that require explicit user interaction in the launcher UI. */
const CLIENT_TOOLS = [ASK_USER_TOOL, SHOW_MODPACK_TOOL] as const;

/** Options for the local Claude Code engine. */
export interface ClaudeCodeEngineOptions {
  /** Anthropic model id for the runtime (defaults to the CLI's own default). */
  model?: string;
}

/**
 * Create a modpack agent whose turns run on the locally-installed Claude Code
 * runtime (subscription login, no API key). Same `ModpackAgent` contract as
 * `createModpackAgent`, plus `dispose()` to end the runtime session.
 */
export function createClaudeCodeModpackAgent(
  handlers: ClientToolHandlers = {},
  options: ClaudeCodeEngineOptions = {},
): ModpackAgent & { dispose: () => Promise<void> } {
  const toolSet = buildTools();
  for (const [name, impl] of Object.entries(handlers)) {
    if (toolSet[name]) toolSet[name] = { ...toolSet[name], execute: (args: unknown) => impl(args) } as never;
  }

  // Interactive UI tools must be removed when a headless host cannot handle
  // them, otherwise the runtime can pause forever waiting for a user action.
  let textFallback = false;
  for (const name of CLIENT_TOOLS) {
    if (!handlers[name]) {
      delete toolSet[name];
      textFallback = true;
    }
  }

  const agent = new HarnessAgent({
    harness: createClaudeCode(options.model ? { model: options.model } : {}),
    sandbox: createLocalSandbox(),
    // Boundary cast: these are ai@6 `tool()` objects fed to an ai@7 consumer.
    // At runtime both are { description, inputSchema(zod), execute } — the
    // harness reads the zod schema via asSchema, which accepts both.
    tools: toolSet as never,
    instructions: CHAT_AGENT_SYSTEM_PROMPT + (textFallback ? TEXT_FALLBACK_NOTE : ""),
    // Constructor-level activeTools drives builtin-tool filtering: naming only
    // our tools denies the runtime's own coding tools (bash/read/write/…).
    activeTools: Object.keys(toolSet) as never,
  });

  let session: Awaited<ReturnType<typeof agent.createSession>> | undefined;

  async function run(
    history: UIMessage[],
    onUpdate: (assistant: UIMessage) => void,
    signal?: AbortSignal,
  ): Promise<TurnResult> {
    return runUiMessageTurn({
      history,
      onUpdate,
      signal,
      start: async () => {
        session ??= await agent.createSession();
        const lastUser = [...history].reverse().find((m) => m.role === "user");
        const prompt = (lastUser?.parts ?? [])
          .map((p) => (p.type === "text" ? p.text : ""))
          .join("");
        return agent.stream({ session, prompt, abortSignal: signal });
      },
      readUIMessageStream,
      // ai@7 UIMessage → the ai@6-typed contract; the part shapes we render
      // (text / reasoning / tool-* state machine) are identical.
      mapMessage: (msg) => msg as unknown as UIMessage,
    });
  }

  return {
    run,
    dispose: async () => {
      await session?.destroy();
      session = undefined;
    },
  };
}
