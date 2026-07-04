#!/usr/bin/env node
// mc-agent — a headless test harness for the @kobemc/agent-core brain.
//
//   mc-agent chat "<prompt>" [--engine openrouter|claude-code]
//                            [--model X] [--json] [--turns "msg1||msg2"]
//
// Streams TextDelta to stdout live and prints tool-call chips. This CLI is
// headless, so launcher client tools are NOT executed here; a tool call is the
// expected stopping point unless the selected runtime supplies its own bridge.
// With --json it prints one AgentStreamEvent per line (pi-style print/JSON
// mode).
//
// --engine openrouter (default): LLM endpoint comes from the env:
//   OPENROUTER_API_KEY   (required for a real turn)
//   OPENROUTER_MODEL     (or --model; default openai/gpt-4o-mini)
//   OPENROUTER_BASE_URL  (default https://openrouter.ai/api/v1)
// A repo-root .env is loaded too when present (via node:process loadEnvFile).
//
// --engine claude-code: NO API key — turns run on the locally-installed
// Claude Code runtime (its subscription login) via the harness engine;
// --model then takes an Anthropic model id (default: the CLI's own default).
//
// The core is TypeScript source (no build step), so we register tsx before
// importing it — this file therefore runs under plain `node`, no flags needed.
import { register } from "tsx/esm/api";
import process from "node:process";

register();

const { createModpackAgent } = await import(new URL("../src/index.ts", import.meta.url).href);

// --- args ---------------------------------------------------------------------

const argv = process.argv.slice(2);
if (argv[0] !== "chat") {
  process.stderr.write(
    'usage: mc-agent chat "<prompt>" [--engine openrouter|claude-code] [--model X] [--json] [--turns "a||b"]\n',
  );
  process.exit(1);
}

const flags = { engine: "openrouter", model: "", json: false, turns: "" };
const positional = [];
for (let i = 1; i < argv.length; i++) {
  const arg = argv[i];
  if (arg === "--json") flags.json = true;
  else if (arg === "--engine") flags.engine = argv[++i];
  else if (arg === "--tools" || arg === "--executor") i++; // legacy no-op
  else if (arg === "--model") flags.model = argv[++i];
  else if (arg === "--turns") flags.turns = argv[++i];
  else positional.push(arg);
}
if (flags.engine !== "openrouter" && flags.engine !== "claude-code") {
  process.stderr.write(`error: unknown --engine "${flags.engine}" (openrouter|claude-code)\n`);
  process.exit(1);
}

const messages = flags.turns
  ? flags.turns.split("||").map((s) => s.trim()).filter(Boolean)
  : positional.length
    ? [positional.join(" ")]
    : [];
if (messages.length === 0) {
  process.stderr.write('error: no prompt. Pass a prompt or --turns "a||b".\n');
  process.exit(1);
}

// --- env / .env ---------------------------------------------------------------

// Best-effort: load repo-root or cwd .env into process.env (Node stdlib; no dep).
for (const candidate of [
  new URL("../../../.env", import.meta.url), // repo root, from packages/agent-core/bin
  new URL(".env", `file://${process.cwd()}/`),
]) {
  try {
    process.loadEnvFile(candidate);
  } catch {
    /* absent / unreadable — env vars alone are fine */
  }
}

const settings = {
  apiKey: process.env.OPENROUTER_API_KEY ?? "",
  model:
    flags.model ||
    process.env.OPENROUTER_MODEL ||
    process.env.MC_AGENT_OPENROUTER_MODEL ||
    "deepseek/deepseek-v4-pro", // align with mc-core's DEFAULT_OPENROUTER_MODEL
  baseUrl: process.env.OPENROUTER_BASE_URL ?? "https://openrouter.ai/api/v1",
};

// --- run ----------------------------------------------------------------------

const clip = (s, n = 200) => (s.length > n ? s.slice(0, n) + "…" : s);

// `agent.run(history, onUpdate)` calls onUpdate with the whole growing assistant
// UIMessage on each stream tick (text/reasoning/tool parts with an
// input-streaming → available → output state machine). We diff it into
// pi-style events so --json stays an event stream and pretty mode reads live.
function emit(ev) {
  if (flags.json) {
    process.stdout.write(JSON.stringify(ev) + "\n");
    return;
  }
  switch (ev.type) {
    case "text_delta":
      process.stdout.write(ev.delta);
      break;
    case "reasoning_delta":
      break; // not surfaced in pretty mode
    case "ask_user":
      process.stdout.write(`\n⏸  ask_user_question — turn paused for user input (client-side tool):\n`);
      process.stdout.write(`   Q: ${ev.question}\n`);
      ev.options.forEach((o, k) => process.stdout.write(`   ${k + 1}. ${o}\n`));
      break;
    case "tool_call":
      process.stdout.write(`\n🔧 ${ev.name}(${clip(JSON.stringify(ev.args ?? {}))})\n`);
      break;
    case "tool_result":
      process.stdout.write(`✓ ${ev.name}: ${clip(ev.summary)}\n`);
      break;
    case "error":
      process.stderr.write(`\n[error] ${ev.message}\n`);
      break;
  }
}

// One diff cursor per turn: how much of each text/reasoning part we've printed,
// and which tool calls/results we've already surfaced.
function makeOnUpdate() {
  const printed = new Map(); // part index -> chars already emitted
  const seen = new Set(); // "<toolCallId>:call" / ":result"
  return (assistant) => {
    assistant.parts.forEach((part, idx) => {
      if (part.type === "text" || part.type === "reasoning") {
        const prev = printed.get(idx) ?? 0;
        const text = part.text ?? "";
        if (text.length > prev) {
          emit({ type: part.type === "text" ? "text_delta" : "reasoning_delta", delta: text.slice(prev) });
          printed.set(idx, text.length);
        }
        return;
      }
      if (typeof part.toolCallId !== "string") return;
      const name = part.type.replace(/^tool-/, "");
      if (part.state === "input-available" && !seen.has(`${part.toolCallId}:call`)) {
        seen.add(`${part.toolCallId}:call`);
        if (name === "ask_user_question") {
          const inp = part.input ?? {};
          emit({
            type: "ask_user",
            question: inp.question ?? "",
            options: (Array.isArray(inp.options) ? inp.options : []).map((o) => o?.label ?? String(o)),
          });
        } else {
          emit({ type: "tool_call", name, args: part.input });
        }
      }
      if (part.state === "output-available" && !seen.has(`${part.toolCallId}:result`)) {
        seen.add(`${part.toolCallId}:result`);
        emit({ type: "tool_result", name, summary: clip(JSON.stringify(part.output ?? {})) });
      }
    });
  };
}

let uid = 0;
const nextId = () => `m${++uid}`;
let agent;
if (flags.engine === "claude-code") {
  const { createClaudeCodeModpackAgent } = await import(
    new URL("../src/harness/index.ts", import.meta.url).href
  );
  agent = createClaudeCodeModpackAgent({}, flags.model ? { model: flags.model } : {});
} else {
  agent = createModpackAgent(settings);
}
let history = [];
for (const [i, msg] of messages.entries()) {
  if (!flags.json) process.stdout.write(`\n${i > 0 ? "\n" : ""}› ${msg}\n`);
  history = [...history, { id: nextId(), role: "user", parts: [{ type: "text", text: msg }] }];
  const res = await agent.run(history, makeOnUpdate());
  if (res.error) emit({ type: "error", message: res.error });
  history = res.messages;
}
await agent.dispose?.();
if (!flags.json) process.stdout.write("\n");
