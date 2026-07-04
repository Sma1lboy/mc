// Local-runtime host adapter: the same `ModpackAgent` contract, but turns run
// on the locally-installed Claude Code (subscription login, no API key).
//
// The engine itself lives in a Node host process (`harness-host.mjs`) — a
// webview can't spawn processes — reached through three thin Tauri commands:
// agent_host_start / agent_host_send / agent_host_stop, with the host's stdout
// protocol lines coming back as `agent-host://event`. Tool calls are proxied
// back over stdio and handled by the launcher client: automatic tools go
// through Rust IPC; ask/show wait for UI clicks.
//
// Session note: the runtime (not this adapter) owns conversation context. We
// send only the newest user text per turn; when the webview's history diverges
// from what this adapter last saw (new chat / switched conversation), we ask
// the host to `reset` — the runtime then starts fresh WITHOUT the old context
// (a switched-to old conversation continues, but the model won't remember its
// earlier turns; known v1 limit of the local-runtime path).

import { listen } from "@tauri-apps/api/event";
import type { UIMessage } from "ai";
import type { ModpackAgent } from "@kobemc/agent-core";
import { commands, type AgentHostEvent } from "../ipc/bindings";
import { runLauncherClientTool, unwrap } from "./clientToolDispatcher";
import { registerLocalClientTool, clearLocalClientTools } from "./chatStore";

type HostMsg =
  | { type: "update"; message: UIMessage }
  | { type: "tool_call"; id: string; name: string; args: unknown }
  | { type: "done"; error?: string }
  | { type: "host_exit" };

interface ActiveTurn {
  history: UIMessage[];
  onUpdate: (assistant: UIMessage) => void;
  assistant?: UIMessage;
  finish: (error?: string) => void;
}

export async function createLocalRuntimeAgent(): Promise<ModpackAgent> {
  await unwrap(commands.agentHostStart());

  let active: ActiveTurn | null = null;
  /** The history as of the end of the last turn — divergence means reset. */
  let lastMessages: UIMessage[] = [];

  const send = (msg: unknown) => unwrap(commands.agentHostSend(JSON.stringify(msg)));

  function handle(msg: HostMsg): void {
    switch (msg.type) {
      case "tool_call": {
        const run =
          msg.name === "ask_user_question" || msg.name === "show_modpack"
            ? new Promise((resolve) => registerLocalClientTool(msg.name, resolve))
            : runLauncherClientTool(msg.name, msg.args);
        void run.then(
          (result) => send({ type: "tool_result", id: msg.id, ok: true, result }),
          (e) => send({ type: "tool_result", id: msg.id, ok: false, error: String(e) }),
        );
        return;
      }
      case "update":
        if (active) {
          active.assistant = msg.message;
          active.onUpdate(msg.message);
        }
        return;
      case "done":
        active?.finish(msg.error);
        return;
      case "host_exit":
        active?.finish("local agent host exited");
        return;
    }
  }

  await listen<AgentHostEvent>("agent-host://event", (e) => {
    try {
      handle(JSON.parse(e.payload.line) as HostMsg);
    } catch {
      /* non-JSON noise — ignore */
    }
  });

  async function run(
    history: UIMessage[],
    onUpdate: (assistant: UIMessage) => void,
    signal?: AbortSignal,
  ): Promise<{ messages: UIMessage[]; error?: string }> {
    const lastUser = [...history].reverse().find((m) => m.role === "user");
    const text = (lastUser?.parts ?? [])
      .map((p) => (p.type === "text" ? p.text : ""))
      .join("");
    // Same conversation ⇔ everything before the new user message is exactly
    // what the last turn ended with. Anything else (fresh chat, switched
    // conversation) → reset the runtime session.
    const prior = history.slice(0, -1);
    const reset =
      prior.length !== lastMessages.length ||
      prior.some((m, i) => m.id !== lastMessages[i]?.id);

    const abort = () => void send({ type: "abort" }).catch(() => {});
    signal?.addEventListener("abort", abort, { once: true });
    try {
      const error = await new Promise<string | undefined>((resolve, reject) => {
        active = { history, onUpdate, finish: resolve };
        send({ type: "turn", text, reset }).catch(reject);
      });
      const assistant = active?.assistant;
      const messages = assistant ? [...history, assistant] : history;
      lastMessages = messages;
      // A user interrupt is a clean stop, not a failure (mirror the API engine).
      return { messages, error: signal?.aborted ? undefined : error };
    } catch (e) {
      lastMessages = history;
      return { messages: history, error: e instanceof Error ? e.message : String(e) };
    } finally {
      signal?.removeEventListener("abort", abort);
      active = null;
      clearLocalClientTools(); // 中断/结束后不能留悬空的待答工具
    }
  }

  return { run };
}
