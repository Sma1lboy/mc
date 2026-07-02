// Deterministic in-process mock of OpenRouter's POST /chat/completions (SSE),
// ported from bench/mock-openrouter.mjs. Drives the brain with zero real network.
//
// startMockServer(opts) resolves to { url, port, close }, where `url` is the
// OpenAI-compatible base (…/v1) to hand createModpackAgent as `baseUrl`.
//
// Scenarios:
//   "text" — stream `chunks` content chunks, then stop.
//   "tool" — the FIRST request of a turn returns ONE tool_call; once a request
//            carries a tool-result message, it returns the text stream. Exactly
//            one tool round-trip, so a test can assert dispatch + feedback.
//
// opts: { scenario, chunks, toolName, toolArgs } — toolName/toolArgs shape the
// single tool_call the "tool" scenario emits (default search_base_modpacks).

import http from "node:http";

function sseChunk(delta, finish = null, extra = {}) {
  const payload = {
    id: "chatcmpl-mock",
    object: "chat.completion.chunk",
    created: Math.floor(Date.now() / 1000),
    model: "mock",
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

function streamText(res, chunks) {
  res.write(sseChunk({ role: "assistant", content: "" }));
  for (let i = 0; i < chunks; i++) {
    res.write(sseChunk({ content: `chunk${i} ` }));
  }
  res.write(sseChunk({}, "stop"));
  res.write("data: [DONE]\n\n");
  res.end();
}

function streamToolCall(res, name, args) {
  const call = {
    tool_calls: [
      {
        index: 0,
        id: "call_mock_1",
        type: "function",
        function: { name, arguments: JSON.stringify(args) },
      },
    ],
  };
  res.write(sseChunk({ role: "assistant" }));
  res.write(sseChunk(call, "tool_calls"));
  res.write("data: [DONE]\n\n");
  res.end();
}

export function startMockServer(opts = {}) {
  const scenario = opts.scenario ?? "text";
  const chunks = opts.chunks ?? 8;
  const toolName = opts.toolName ?? "search_base_modpacks";
  const toolArgs = opts.toolArgs ?? { query: "tech" };

  const server = http.createServer(async (req, res) => {
    const body = await readBody(req);
    let parsed = {};
    try {
      parsed = JSON.parse(body || "{}");
    } catch {
      /* ignore */
    }
    const msgs = Array.isArray(parsed.messages) ? parsed.messages : [];
    const hadToolResult = msgs.some((m) => m && m.role === "tool");

    res.writeHead(200, {
      "content-type": "text/event-stream",
      "cache-control": "no-cache",
      connection: "close",
    });

    if (scenario === "tool" && !hadToolResult) {
      streamToolCall(res, toolName, toolArgs);
    } else {
      streamText(res, chunks);
    }
  });

  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      resolve({
        port,
        url: `http://127.0.0.1:${port}/v1`,
        close: () => new Promise((r) => server.close(r)),
      });
    });
  });
}
