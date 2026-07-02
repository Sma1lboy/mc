// Orchestrator: for each streaming cell (0ms and 20ms per-chunk delay) it starts
// the mock, drives BOTH brains (K warmup + N measured turns, same prompt, same
// 60×8-token text scenario), wraps each brain in `/usr/bin/time -l` for peak RSS,
// and reduces everything to median + p10/p90. Also runs a one-tool-round-trip cell
// to compare rig vs ai-sdk request counts, and the serde micro-bench. Writes
// bench/results.json and prints a summary. NO real network.
//
// Run: node bench/run-all.mjs   (from the worktree root)

import { spawn } from "node:child_process";
import { writeFileSync } from "node:fs";
import { setTimeout as delay } from "node:timers/promises";

const ROOT = "/Users/jacksonc/i/mc-launcher/.claude/worktrees/agent-ts-brain";
const DESKTOP = `${ROOT}/desktop`;
const BENCH = `${ROOT}/bench`;
const MC_BIN = `${ROOT}/target/debug/mc`;
const CORE = `${DESKTOP}/src/agent/core/agent.ts`;
const PORT = 8799;
const WARMUP = 3;
const MEASURED = 12;
const CHUNKS = 60;
const TOKENS_PER_CHUNK = 8;

// ---- small helpers ----------------------------------------------------------
function sorted(a) {
  return [...a].sort((x, y) => x - y);
}
function pct(a, q) {
  const s = sorted(a);
  if (s.length === 0) return null;
  const i = Math.round(q * (s.length - 1));
  return s[Math.max(0, Math.min(s.length - 1, i))];
}
function stat(a) {
  return { p10: r2(pct(a, 0.1)), median: r2(pct(a, 0.5)), p90: r2(pct(a, 0.9)) };
}
function r2(x) {
  return x == null ? null : Math.round(x * 100) / 100;
}
function col(rows, key) {
  return rows.map((r) => r[key]);
}

// Run a command; resolve { stdout, stderr, code }.
function run(cmd, args, opts = {}) {
  return new Promise((resolve) => {
    const child = spawn(cmd, args, opts);
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("close", (code) => resolve({ stdout, stderr, code }));
  });
}

// Peak RSS (bytes) from `/usr/bin/time -l` stderr, macOS format.
function parseMaxRss(stderr) {
  const m = stderr.match(/(\d+)\s+maximum resident set size/);
  return m ? Number(m[1]) : null;
}

async function startMock(env) {
  const child = spawn("node", [`${BENCH}/mock-openrouter.mjs`], {
    cwd: BENCH,
    env: { ...process.env, PORT: String(PORT), ...env },
  });
  // Wait for the ready line.
  await new Promise((resolve) => {
    child.stdout.once("data", () => resolve());
  });
  return child;
}
async function stopMock(child) {
  child.kill("SIGTERM");
  await delay(150);
}
async function reset() {
  await fetch(`http://127.0.0.1:${PORT}/__reset`);
}
async function getLog() {
  return (await fetch(`http://127.0.0.1:${PORT}/__log`)).json();
}

// ---- one text cell (both brains) -------------------------------------------
async function textCell(delayMs) {
  const mock = await startMock({ SCENARIO: "text", CHUNKS: String(CHUNKS), TOKENS_PER_CHUNK: String(TOKENS_PER_CHUNK), DELAY_MS: String(delayMs) });

  // TS, wrapped in /usr/bin/time -l for peak RSS.
  const ts = await run(
    "/usr/bin/time",
    ["-l", "node", "--import", "tsx", `${BENCH}/run-ts.mjs`],
    { cwd: DESKTOP, env: { ...process.env, PORT: String(PORT), WARMUP: String(WARMUP), MEASURED: String(MEASURED), CORE } },
  );
  const tsJson = JSON.parse(ts.stdout.trim().split("\n").pop());
  const tsRss = parseMaxRss(ts.stderr);

  // Rust timed loop (node driver, shared clock).
  const rust = await run("node", [`${BENCH}/run-rust.mjs`], {
    cwd: ROOT,
    env: { ...process.env, PORT: String(PORT), WARMUP: String(WARMUP), MEASURED: String(MEASURED), MC_BIN, CWD: ROOT },
  });
  const rustJson = JSON.parse(rust.stdout.trim().split("\n").pop());

  // Rust peak RSS: median of 3 single-turn invocations under /usr/bin/time -l.
  const rustRssSamples = [];
  for (let i = 0; i < 3; i++) {
    const one = await run(
      "/usr/bin/time",
      ["-l", MC_BIN, "agent", "chat", "Recommend a good 1.20.1 Fabric performance modpack."],
      { cwd: ROOT, env: { ...process.env, OPENROUTER_BASE_URL: `http://127.0.0.1:${PORT}/v1`, OPENROUTER_API_KEY: "bench", MC_AGENT_OPENROUTER_MODEL: "bench", MC_LOG: "error" } },
    );
    const rss = parseMaxRss(one.stderr);
    if (rss) rustRssSamples.push(rss);
  }
  const rustRss = pct(rustRssSamples, 0.5);

  await stopMock(mock);

  return {
    delayMs,
    ts: {
      serverTtftMs: stat(col(tsJson.rows, "serverTtft")),
      clientTtftMs: stat(col(tsJson.rows, "clientTtft")),
      serverTotalMs: stat(col(tsJson.rows, "serverTotal")),
      clientTotalMs: stat(col(tsJson.rows, "clientTotal")),
      deltasPerSec: stat(col(tsJson.rows, "deltasPerSec")),
      rssBytes_timeL: tsRss,
      rssBytes_inProcPeak: tsJson.rssBytesPeakInProc,
      heapUsedBytesFinal: tsJson.heapUsedBytesFinal,
    },
    rust: {
      serverTtftMs: stat(col(rustJson.rows, "serverTtft")),
      serverTotalMs: stat(col(rustJson.rows, "serverTotal")),
      spawnOverheadMs: stat(col(rustJson.rows, "spawnOverhead")),
      processWallMs: stat(col(rustJson.rows, "processWall")),
      rssBytes_timeL_median: rustRss,
      rssBytes_timeL_samples: rustRssSamples,
    },
  };
}

// ---- tool round-trip: request-count comparison ------------------------------
async function toolCell() {
  const mock = await startMock({ SCENARIO: "tool", CHUNKS: "12", TOKENS_PER_CHUNK: "4", DELAY_MS: "0" });
  const out = {};

  await reset();
  await run("node", ["--import", "tsx", `${BENCH}/run-ts.mjs`], {
    cwd: DESKTOP,
    env: { ...process.env, PORT: String(PORT), WARMUP: "0", MEASURED: "1", CORE },
  });
  out.tsRequests = await getLog();

  await reset();
  await run("node", [`${BENCH}/run-rust.mjs`], {
    cwd: ROOT,
    env: { ...process.env, PORT: String(PORT), WARMUP: "0", MEASURED: "1", MC_BIN, CWD: ROOT },
  });
  out.rustRequests = await getLog();

  await stopMock(mock);
  return {
    tsRequestCount: out.tsRequests.length,
    tsRequestKinds: out.tsRequests.map((r) => r.kind),
    rustRequestCount: out.rustRequests.length,
    rustRequestKinds: out.rustRequests.map((r) => r.kind),
  };
}

// ---- serde micro ------------------------------------------------------------
async function serdeMicro() {
  const res = await run("node", [`${BENCH}/serde-payload.mjs`], { cwd: BENCH, env: process.env });
  return JSON.parse(res.stdout.trim());
}

// ---- main -------------------------------------------------------------------
const results = { meta: { warmup: WARMUP, measured: MEASURED, chunks: CHUNKS, tokensPerChunk: TOKENS_PER_CHUNK, node: process.version, when: new Date().toISOString() } };
console.error("cell: text, delay 0ms ...");
results.cell_0ms = await textCell(0);
console.error("cell: text, delay 20ms ...");
results.cell_20ms = await textCell(20);
console.error("cell: tool round-trip ...");
results.tool = await toolCell();
console.error("serde micro ...");
results.serde = await serdeMicro();

writeFileSync(`${BENCH}/results.json`, JSON.stringify(results, null, 2));
console.error("\nwrote bench/results.json");
console.log(JSON.stringify(results, null, 2));
