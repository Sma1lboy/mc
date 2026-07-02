// ONE text-only turn through the TS brain against REAL OpenRouter — end-to-end
// proof the streaming path works outside the mock, plus a real TTFT/turn data point
// for color. NOT looped (costs money). Needs OPENROUTER_API_KEY in env.
//   cd desktop && node --import tsx <bench>/smoke-real.mjs
//
// Model: MC_AGENT_OPENROUTER_MODEL || OPENROUTER_MODEL || deepseek/deepseek-v4-pro.

import { pathToFileURL } from "node:url";

const CORE = "/Users/jacksonc/i/mc-launcher/.claude/worktrees/agent-ts-brain/desktop/src/agent/core/agent.ts";
const absMs = () => performance.timeOrigin + performance.now();

const apiKey = process.env.OPENROUTER_API_KEY;
if (!apiKey) throw new Error("OPENROUTER_API_KEY not set");
const model = process.env.MC_AGENT_OPENROUTER_MODEL || process.env.OPENROUTER_MODEL || "deepseek/deepseek-v4-pro";

const { createModpackAgent } = await import(pathToFileURL(CORE).href);

// Instant tool backend (in case the model calls one); prompt asks for text only.
const toolExec = new Proxy({}, { get: () => async () => ({ ok: true, items: [] }) });
const agent = createModpackAgent({ apiKey, model, baseUrl: "https://openrouter.ai/api/v1" }, toolExec);

let tStart = absMs();
let tFirst = null;
let tDone = null;
let deltas = 0;
let toolCalls = 0;
let errors = [];

const { reply } = await agent.runTurn([], "In one sentence, greet me and say what you help with. Do not call any tools.", (ev) => {
  if (ev.type === "text_delta") {
    if (tFirst === null) tFirst = absMs();
    deltas++;
  } else if (ev.type === "tool_call") {
    toolCalls++;
  } else if (ev.type === "error") {
    errors.push(ev.message);
  } else if (ev.type === "done") {
    tDone = absMs();
  }
});

console.log(
  JSON.stringify(
    {
      model,
      ttftMs: tFirst ? Math.round((tFirst - tStart) * 10) / 10 : null,
      totalMs: tDone ? Math.round((tDone - tStart) * 10) / 10 : null,
      deltas,
      toolCalls,
      errors,
      replyChars: reply.length,
      replyPreview: reply.slice(0, 200),
    },
    null,
    2,
  ),
);
