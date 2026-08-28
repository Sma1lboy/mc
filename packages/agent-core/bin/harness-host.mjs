#!/usr/bin/env node
// Long-lived local Claude runtime host. Rust/Tauri forwards these JSON lines
// unchanged; `harness-host-router.mjs` owns per-conversation sessions.
import { register } from "tsx/esm/api";
import process from "node:process";
import {
  createHarnessHostLineReader,
  createHarnessHostRouter,
} from "./harness-host-router.mjs";

register();

const { createClaudeCodeModpackAgent } = await import(
  new URL("../src/harness/index.ts", import.meta.url).href
);

const router = createHarnessHostRouter({
  model: process.env.MC_AGENT_CLAUDE_MODEL || undefined,
  send: (message) => process.stdout.write(`${JSON.stringify(message)}\n`),
  createAgent: (handlers, options) => createClaudeCodeModpackAgent(handlers, options),
});

let stopping = false;
function stop(code) {
  if (stopping) return;
  stopping = true;
  process.stdin.pause();
  void router.dispose().finally(() => process.exit(code));
}

const reader = createHarnessHostLineReader({
  onMessage: (message) => {
    if (message?.type === "dispose") {
      stop(0);
      return;
    }
    router.handle(message);
  },
  onBadLine: (line) => process.stderr.write(`harness-host: bad line: ${line}\n`),
  onFatal: (error) => {
    process.stderr.write(`harness-host: ${error.code}: ${error.message}\n`);
    stop(1);
  },
});

process.stdin.on("data", (chunk) => reader.push(chunk));
process.stdin.on("end", () => {
  reader.end();
  if (!stopping) stop(0);
});
process.stdin.on("error", (error) => {
  if (stopping) return;
  process.stderr.write(`harness-host: stdin error: ${error.message}\n`);
  stop(1);
});

process.stderr.write("harness-host: ready\n");
