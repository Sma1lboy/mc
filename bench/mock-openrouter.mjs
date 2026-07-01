// Deterministic mock of OpenRouter's POST /chat/completions (stream:true), used
// to drive BOTH agent brains (rig / ai-sdk) through identical work with zero real
// network. It also doubles as the shared timing oracle: it records the absolute
// (sub-ms) wall-clock instant it *received* each request, and serves that log at
// GET /__log. Both harnesses read the same clock (performance.timeOrigin +
// performance.now()), so TTFT can be anchored on "request received" — which
// cancels the Rust CLI's process-spawn + config-load cost for a fair comparison.
//
// Path is ignored: rig posts to {base}/chat/completions, ai-sdk to
// {base}/chat/completions too, and we answer any POST with SSE. GET paths starting
// with /__ are control endpoints.
//
// Scenario (env):
//   PORT                port to listen on (default 8799)
//   SCENARIO            "text" (default) | "tool"
//   CHUNKS              number of streamed content chunks (default 60)
//   TOKENS_PER_CHUNK    ~tokens (short words) per chunk (default 8)
//   DELAY_MS            per-chunk delay before each content chunk (default 0)
//
// In "tool" scenario the first request of a turn returns ONE tool_call; once a
// tool-result message is present in the request, it returns the text stream. This
// yields exactly one tool round-trip so we can compare rig vs ai-sdk request counts.

import http from "node:http";

const PORT = Number(process.env.PORT || 8799);
const SCENARIO = process.env.SCENARIO || "text";
const CHUNKS = Number(process.env.CHUNKS || 60);
const TOKENS_PER_CHUNK = Number(process.env.TOKENS_PER_CHUNK || 8);
const DELAY_MS = Number(process.env.DELAY_MS || 0);

// Shared high-res absolute clock (epoch ms with sub-ms precision).
const absMs = () => performance.timeOrigin + performance.now();
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// One "token": a short word (~1 BPE token). Content of one chunk = M of them.
const chunkContent = (i) =>
  Array.from({ length: TOKENS_PER_CHUNK }, (_, k) => `t${i}.${k}`).join(" ") + " ";

let requestLog = []; // { index, tReceived, scenario, kind, hadToolResult }
let reqCounter = 0;

function sseChunk(delta, finish = null, extra = {}) {
  const payload = {
    id: "chatcmpl-bench",
    object: "chat.completion.chunk",
    created: Math.floor(Date.now() / 1000),
    model: "bench",
    choices: [{ index: 0, delta, finish_reason: finish }],
    ...extra,
  };
  return `data: ${JSON.stringify(payload)}\n\n`;
}

function readBody(req) {
  return new Promise((resolve) => {
    let buf = "";
    req.on("data", (c) => (buf += c));
    req.on("end", () => resolve(buf));
  });
}

async function streamText(res) {
  // First chunk carries role; subsequent chunks carry content only.
  res.write(sseChunk({ role: "assistant", content: "" }));
  for (let i = 0; i < CHUNKS; i++) {
    if (DELAY_MS > 0) await sleep(DELAY_MS);
    res.write(sseChunk({ content: chunkContent(i) }));
  }
  res.write(sseChunk({}, "stop"));
  res.write("data: [DONE]\n\n");
  res.end();
}

async function streamToolCall(res) {
  // A single tool_call, streamed the way OpenAI/OpenRouter deliver them.
  const call = {
    tool_calls: [
      {
        index: 0,
        id: "call_bench_1",
        type: "function",
        function: {
          name: "search_mods",
          arguments: JSON.stringify({ query: "performance", mc_version: "1.20.1", loader: "fabric" }),
        },
      },
    ],
  };
  res.write(sseChunk({ role: "assistant" }));
  res.write(sseChunk(call, "tool_calls"));
  res.write("data: [DONE]\n\n");
  res.end();
}

const server = http.createServer(async (req, res) => {
  const tReceived = absMs();

  if (req.method === "GET" && req.url.startsWith("/__log")) {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify(requestLog));
    return;
  }
  if (req.method === "GET" && req.url.startsWith("/__reset")) {
    requestLog = [];
    reqCounter = 0;
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ ok: true }));
    return;
  }
  if (req.method === "GET" && req.url.startsWith("/__health")) {
    res.writeHead(200, { "content-type": "application/json" });
    res.end(JSON.stringify({ ok: true, scenario: SCENARIO, chunks: CHUNKS, tokensPerChunk: TOKENS_PER_CHUNK, delayMs: DELAY_MS }));
    return;
  }

  const body = await readBody(req);
  let parsed = {};
  try {
    parsed = JSON.parse(body || "{}");
  } catch {
    /* ignore */
  }
  const messages = Array.isArray(parsed.messages) ? parsed.messages : [];
  const hadToolResult = messages.some((m) => m && m.role === "tool");
  const wantsUsage = parsed.stream_options && parsed.stream_options.include_usage === true;

  const index = reqCounter++;
  const kind = SCENARIO === "tool" && !hadToolResult ? "tool_call" : "text";
  requestLog.push({ index, tReceived, scenario: SCENARIO, kind, hadToolResult });

  res.writeHead(200, {
    "content-type": "text/event-stream",
    "cache-control": "no-cache",
    connection: "close",
  });

  if (kind === "tool_call") {
    await streamToolCall(res);
  } else {
    await streamText(res);
    if (wantsUsage) {
      // OpenRouter emits a trailing usage-only chunk when include_usage is set.
      res.write(sseChunk({}, null, { choices: [], usage: { prompt_tokens: 12, completion_tokens: CHUNKS * TOKENS_PER_CHUNK, total_tokens: 12 + CHUNKS * TOKENS_PER_CHUNK } }));
    }
  }
});

server.listen(PORT, "127.0.0.1", () => {
  // Machine-readable ready line so harnesses can wait for it.
  process.stdout.write(
    JSON.stringify({ ready: true, port: PORT, scenario: SCENARIO, chunks: CHUNKS, tokensPerChunk: TOKENS_PER_CHUNK, delayMs: DELAY_MS }) + "\n",
  );
});
