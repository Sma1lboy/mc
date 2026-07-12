#!/usr/bin/env node
// Long-lived local Claude runtime host. Rust/Tauri forwards these JSON lines
// unchanged; `harness-host-router.mjs` owns per-conversation sessions.
import { register } from "tsx/esm/api";
import readline from "node:readline";
import process from "node:process";
import { createHarnessHostRouter } from "./harness-host-router.mjs";

register();

const { createClaudeCodeModpackAgent } = await import(
  new URL("../src/harness/index.ts", import.meta.url).href
);

const router = createHarnessHostRouter({
  model: process.env.MC_AGENT_CLAUDE_MODEL || undefined,
  send: (message) => process.stdout.write(`${JSON.stringify(message)}\n`),
  createAgent: (handlers, options) => createClaudeCodeModpackAgent(handlers, options),
});

const rl = readline.createInterface({ input: process.stdin });
rl.on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    process.stderr.write(`harness-host: bad line: ${line.slice(0, 200)}\n`);
    return;
  }
  if (message.type === "dispose") {
    void router.dispose().finally(() => process.exit(0));
    return;
  }
  router.handle(message);
});

rl.on("close", () => {
  void router.dispose().finally(() => process.exit(0));
});

process.stderr.write("harness-host: ready\n");
