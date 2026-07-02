// Drives the TS brain (desktop/src/agent/core) in plain Node against the mock.
// Run under tsx so the .ts core (and its `ai`/`zod` imports from desktop/) load:
//   cd desktop && node --import tsx <bench>/run-ts.mjs
//
// Emits ONE JSON object on stdout (the only stdout output) so an outer runner can
// wrap this in `/usr/bin/time -l` and read peak RSS from stderr without collision.
//
// Env: PORT (mock), WARMUP (default 3), MEASURED (default 12),
//      CORE (abs path to agent.ts). Everything else is fixed to match Rust.

import { pathToFileURL } from "node:url";

const PORT = Number(process.env.PORT || 8799);
const WARMUP = Number(process.env.WARMUP || 3);
const MEASURED = Number(process.env.MEASURED || 12);
const BASE_URL = `http://127.0.0.1:${PORT}/v1`;
const CORE =
  process.env.CORE ||
  "/Users/jacksonc/i/mc-launcher/.claude/worktrees/agent-ts-brain/desktop/src/agent/core/agent.ts";

const absMs = () => performance.timeOrigin + performance.now();
const PROMPT = "Recommend a good 1.20.1 Fabric performance modpack.";

// Instant, side-effect-free tool backend so a tool round-trip (if any) adds no work.
const toolExec = new Proxy(
  {},
  { get: () => async () => ({ ok: true, items: [] }) },
);

const { createModpackAgent } = await import(pathToFileURL(CORE).href);

async function oneTurn(agent) {
  let tStart = absMs();
  let tFirst = null;
  let tLast = null;
  let deltas = 0;
  await agent.runTurn([], PROMPT, (ev) => {
    if (ev.type === "text_delta") {
      if (tFirst === null) tFirst = absMs();
      tLast = absMs();
      deltas++;
    }
  });
  const tDone = absMs();
  return { tStart, tFirst, tLast, tDone, deltas };
}

async function getLog() {
  const r = await fetch(`http://127.0.0.1:${PORT}/__log`);
  return r.json();
}
async function resetLog() {
  await fetch(`http://127.0.0.1:${PORT}/__reset`);
}

const settings = { apiKey: "bench", model: "bench", baseUrl: BASE_URL };
const agent = createModpackAgent(settings, toolExec);

// Warm up (JIT, connection setup), then reset the mock log so request[i] == turn[i].
for (let i = 0; i < WARMUP; i++) await oneTurn(agent);
await resetLog();

const turns = [];
let peakRss = 0;
for (let i = 0; i < MEASURED; i++) {
  turns.push(await oneTurn(agent));
  const rss = process.memoryUsage().rss;
  if (rss > peakRss) peakRss = rss;
}

const log = await getLog(); // request[i].tReceived aligns with turns[i]
const rows = turns.map((t, i) => {
  const tRecv = log[i] ? log[i].tReceived : t.tStart;
  const streamSec = Math.max((t.tLast - t.tFirst) / 1000, 1e-9);
  return {
    serverTtft: t.tFirst - tRecv, // request-received -> first delta (fair anchor)
    clientTtft: t.tFirst - t.tStart, // in-process runTurn start -> first delta
    serverTotal: t.tDone - tRecv, // request-received -> done
    clientTotal: t.tDone - t.tStart,
    deltasPerSec: t.deltas / streamSec,
    deltas: t.deltas,
  };
});

const mem = process.memoryUsage();
process.stdout.write(
  JSON.stringify({
    brain: "ts",
    port: PORT,
    warmup: WARMUP,
    measured: MEASURED,
    rows,
    rssBytesPeakInProc: peakRss,
    heapUsedBytesFinal: mem.heapUsed,
    rssBytesFinal: mem.rss,
  }) + "\n",
);
