#!/usr/bin/env node
// harness-host — the desktop's local-runtime agent host.
//
// A long-lived Node process the Tauri shell spawns to run the claude-code
// harness engine (webviews can't spawn processes). Line-delimited JSON on
// stdio; the Rust side is a dumb pipe, the webview is the real peer:
//
//   stdin  ← {type:"turn", text, reset?}             start one assistant turn
//                                                    (reset: drop runtime session
//                                                     first — new/switched convo)
//          ← {type:"tool_result", id, ok, result}    answer to a tool_call
//          ← {type:"abort"}                          interrupt the active turn
//          ← {type:"dispose"}                        end session + exit
//   stdout → {type:"update", message}                growing assistant UIMessage
//          → {type:"tool_call", id, name, args}      execute this tool, reply tool_result
//          → {type:"done", error?}                   turn finished
//
// Tools execute in the WEBVIEW (via its existing agent_tool_* bindings — that's
// where activeRoot() etc. live), so these host overrides are a stdio proxy.
// stderr is free-form diagnostics (the Rust side logs it).
import { register } from "tsx/esm/api";
import readline from "node:readline";
import process from "node:process";

register();

const { createClaudeCodeModpackAgent } = await import(
  new URL("../src/harness/index.ts", import.meta.url).href
);

const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");

// --- stdio tool proxy -----------------------------------------------------

let toolSeq = 0;
const pendingTools = new Map(); // id -> {resolve, reject}

function callTool(name, args) {
  const id = `t${++toolSeq}`;
  return new Promise((resolve, reject) => {
    pendingTools.set(id, { resolve, reject });
    send({ type: "tool_call", id, name, args });
  });
}

const TOOL_NAMES = [
  "search_base_modpacks",
  "inspect_base_modpack",
  "search_mods",
  "mod_get_detail",
  "resolve_mods",
  "build_modpack",
  "list_instances",
  "wiki_search",
  "wiki_open",
  // Client-side tools: the webview's override resolves them with the USER's
  // answer (chips click / install click) — the turn stays open while waiting.
  "ask_user_question",
  "show_modpack",
];
const toolHandlers = Object.fromEntries(TOOL_NAMES.map((n) => [n, (args) => callTool(n, args)]));

// --- engine + turn loop ---------------------------------------------------

const model = process.env.MC_AGENT_CLAUDE_MODEL || undefined;
const agent = createClaudeCodeModpackAgent(toolHandlers, model ? { model } : {});

let history = [];
let abort = null;
let running = false;
let uid = 0;

async function runTurn(text, reset) {
  if (running) {
    send({ type: "done", error: "turn already running" });
    return;
  }
  running = true;
  abort = new AbortController();
  if (reset) {
    await agent.dispose(); // next run lazily opens a fresh runtime session
    history = [];
  }
  history = [...history, { id: `u${++uid}`, role: "user", parts: [{ type: "text", text }] }];
  try {
    const res = await agent.run(history, (assistant) => send({ type: "update", message: assistant }), abort.signal);
    history = res.messages;
    send({ type: "done", error: res.error });
  } catch (e) {
    send({ type: "done", error: e instanceof Error ? e.message : String(e) });
  } finally {
    running = false;
    abort = null;
  }
}

// --- stdin dispatch ---------------------------------------------------------

const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    process.stderr.write(`harness-host: bad line: ${line.slice(0, 200)}\n`);
    return;
  }
  switch (msg.type) {
    case "turn":
      void runTurn(String(msg.text ?? ""), msg.reset === true);
      break;
    case "tool_result": {
      const pending = pendingTools.get(msg.id);
      if (!pending) break;
      pendingTools.delete(msg.id);
      if (msg.ok) pending.resolve(msg.result);
      else pending.reject(new Error(String(msg.error ?? "tool failed")));
      break;
    }
    case "abort":
      abort?.abort();
      break;
    case "dispose":
      void agent.dispose().finally(() => process.exit(0));
      break;
    default:
      process.stderr.write(`harness-host: unknown message type: ${String(msg.type)}\n`);
  }
});

// Parent (Rust) died / closed stdin → clean up the runtime session and exit.
rl.on("close", () => {
  void agent.dispose().finally(() => process.exit(0));
});

process.stderr.write("harness-host: ready\n");
