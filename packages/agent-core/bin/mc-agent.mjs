#!/usr/bin/env node
// mc-agent — a headless test harness for the @kobemc/agent-core brain.
//
//   mc-agent chat "<prompt>" [--executor mock|modrinth] [--model X] [--json]
//                            [--turns "msg1||msg2"]
//
// Streams TextDelta to stdout live and prints tool chips like the Rust CLI
// (🔧 name(args) / ✓ name: summary). With --json it prints one AgentStreamEvent
// per line instead (pi-style print/JSON mode). LLM endpoint comes from the env:
//   OPENROUTER_API_KEY   (required for a real turn)
//   OPENROUTER_MODEL     (or --model; default openai/gpt-4o-mini)
//   OPENROUTER_BASE_URL  (default https://openrouter.ai/api/v1)
// A repo-root .env is loaded too when present (via node:process loadEnvFile).
//
// The core is TypeScript source (no build step), so we register tsx before
// importing it — this file therefore runs under plain `node`, no flags needed.
import { register } from "tsx/esm/api";
import process from "node:process";

register();

const { createModpackAgent } = await import(new URL("../src/index.ts", import.meta.url).href);
const { mockExecutor, modrinthExecutor } = await import(
  new URL("../src/executors/index.ts", import.meta.url).href
);

// --- args ---------------------------------------------------------------------

const argv = process.argv.slice(2);
if (argv[0] !== "chat") {
  process.stderr.write(
    'usage: mc-agent chat "<prompt>" [--executor mock|modrinth] [--model X] [--json] [--turns "a||b"]\n',
  );
  process.exit(1);
}

const flags = { executor: "modrinth", model: "", json: false, turns: "" };
const positional = [];
for (let i = 1; i < argv.length; i++) {
  const arg = argv[i];
  if (arg === "--json") flags.json = true;
  else if (arg === "--executor") flags.executor = argv[++i];
  else if (arg === "--model") flags.model = argv[++i];
  else if (arg === "--turns") flags.turns = argv[++i];
  else positional.push(arg);
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
    "openai/gpt-4o-mini",
  baseUrl: process.env.OPENROUTER_BASE_URL ?? "https://openrouter.ai/api/v1",
};

const executor =
  flags.executor === "mock"
    ? mockExecutor()
    : flags.executor === "modrinth"
      ? modrinthExecutor()
      : null;
if (!executor) {
  process.stderr.write(`error: unknown --executor "${flags.executor}" (mock|modrinth)\n`);
  process.exit(1);
}

// --- run ----------------------------------------------------------------------

const clip = (s, n = 200) => (s.length > n ? s.slice(0, n) + "…" : s);

function onEvent(ev) {
  if (flags.json) {
    process.stdout.write(JSON.stringify(ev) + "\n");
    return;
  }
  switch (ev.type) {
    case "text_delta":
      process.stdout.write(ev.delta);
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
    // reasoning / done: not surfaced in pretty mode.
  }
}

const agent = createModpackAgent(settings, executor);
let history = [];
for (const [i, msg] of messages.entries()) {
  if (!flags.json) process.stdout.write(`\n${i > 0 ? "\n" : ""}› ${msg}\n`);
  const res = await agent.runTurn(history, msg, onEvent);
  history = res.history;
}
if (!flags.json) process.stdout.write("\n");
