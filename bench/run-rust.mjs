// Times the Rust brain (rig) via the `mc agent chat` CLI against the mock.
// Node (not shell) because we need a sub-ms absolute clock shared with the mock,
// which macOS `date` can't provide. Spawns the binary once per run; the first
// stdout byte is the first streamed TextDelta (StdoutSink prints nothing before it).
//
// Fair TTFT = mock's request-received instant -> first stdout byte. Because the
// request only hits the mock AFTER the process has spawned + loaded config + built
// the agent, this anchor EXCLUDES the CLI's per-process spawn cost (which the
// desktop app never pays — mc-core is in-process there). Spawn cost is reported
// separately as (request-received - spawn).
//
// Env: PORT, WARMUP (3), MEASURED (12), MC_BIN (abs path to mc), CWD (worktree root).

import { spawn } from "node:child_process";

const PORT = Number(process.env.PORT || 8799);
const WARMUP = Number(process.env.WARMUP || 3);
const MEASURED = Number(process.env.MEASURED || 12);
const MC_BIN =
  process.env.MC_BIN ||
  "/Users/jacksonc/i/mc-launcher/.claude/worktrees/agent-ts-brain/target/debug/mc";
const CWD = process.env.CWD || "/Users/jacksonc/i/mc-launcher/.claude/worktrees/agent-ts-brain";

const absMs = () => performance.timeOrigin + performance.now();
const PROMPT = "Recommend a good 1.20.1 Fabric performance modpack.";

const childEnv = {
  ...process.env,
  OPENROUTER_BASE_URL: `http://127.0.0.1:${PORT}/v1`,
  OPENROUTER_API_KEY: "bench",
  MC_AGENT_OPENROUTER_MODEL: "bench",
  // Silence debug-build stderr tracing so it doesn't compete for the event loop.
  MC_LOG: "error",
};

function oneRun() {
  return new Promise((resolve) => {
    const tSpawn = absMs();
    let tFirst = null;
    let tLast = null;
    const child = spawn(MC_BIN, ["agent", "chat", PROMPT], { cwd: CWD, env: childEnv });
    child.stdout.on("data", () => {
      const now = absMs();
      if (tFirst === null) tFirst = now;
      tLast = now;
    });
    child.stderr.on("data", () => {});
    child.on("close", () => resolve({ tSpawn, tFirst, tLast, tClose: absMs() }));
  });
}

async function getLog() {
  const r = await fetch(`http://127.0.0.1:${PORT}/__log`);
  return r.json();
}
async function resetLog() {
  await fetch(`http://127.0.0.1:${PORT}/__reset`);
}

for (let i = 0; i < WARMUP; i++) await oneRun();
await resetLog();

const runs = [];
for (let i = 0; i < MEASURED; i++) runs.push(await oneRun());

const log = await getLog(); // request[i].tReceived aligns with runs[i] (1 req/turn)
const rows = runs.map((r, i) => {
  const tRecv = log[i] ? log[i].tReceived : r.tSpawn;
  return {
    serverTtft: r.tFirst - tRecv, // request-received -> first stdout byte (fair)
    serverTotal: r.tLast - tRecv, // request-received -> last stdout byte
    spawnOverhead: tRecv - r.tSpawn, // process spawn + config load + connect (informational)
    processWall: r.tClose - r.tSpawn, // whole CLI invocation wall time (informational)
  };
});

process.stdout.write(JSON.stringify({ brain: "rust", port: PORT, warmup: WARMUP, measured: MEASURED, rows }) + "\n");
