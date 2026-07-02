# Rust (rig) vs TS (Vercel AI SDK) agent-brain profiling

**Question.** kobeMC has two implementations of the same streaming tool-use "brain":
the Rust one in `mc-core` (`agent/chat/run.rs`, built on **rig 0.39**) and a
host-agnostic TS one in `desktop/src/agent/core/` (built on the **Vercel AI SDK
`ai` v6** + `@openrouter/ai-sdk-provider`). Both run the *same* turn loop: stream
text/reasoning, auto-dispatch six deterministic tools, feed results back, repeat to
a final answer. This measures whether the TS brain's per-turn overhead is material
**relative to network reality** (an LLM turn is seconds).

## TL;DR

The two brains are indistinguishable in the only dimension a user feels: **the LLM
network round-trip dominates by ~1000x.** Against a zero-latency mock, the whole TS
turn (stream + parse 60 chunks) costs **~3.9 ms** vs Rust's **~2.3 ms** — a ~1.6 ms
gap. A real OpenRouter turn was **1869 ms to first token, 2778 ms total.** So the
framework delta is ~0.06% of a real turn. Memory: the Rust CLI process peaks ~20 MB;
the Node harness peaks ~190 MB (but see the RSS caveat — neither is the in-app
marginal cost). **Recommendation: pick the brain on architecture/maintainability,
not performance. Both are free at network scale.**

## Method

- **Harness** (all in `bench/`): `mock-openrouter.mjs` is a deterministic SSE mock
  of `POST /chat/completions` (`stream:true`) that emits **N content chunks of M
  ~tokens** with an optional per-chunk delay, and records the absolute instant it
  *received* each request. `run-ts.mjs` drives the TS core in Node (via `tsx`) with a
  mock `ToolExecutor`; `run-rust.mjs` spawns the `mc agent chat` CLI per turn;
  `run-all.mjs` orchestrates both across cells and wraps each in `/usr/bin/time -l`
  for peak RSS. Raw numbers in `bench/results.json`.
- **Shared clock.** The mock, the TS driver, and the Rust driver are all Node
  processes on one machine using `performance.timeOrigin + performance.now()`
  (sub-ms absolute epoch). This lets TTFT be **anchored on the mock's
  request-received instant** rather than on process start.
- **Why the request-received anchor matters.** The request only reaches the mock
  *after* a process has spawned, loaded config, and built the agent. Anchoring TTFT
  there **cancels the Rust CLI's per-process spawn cost** (~20 ms) — which the desktop
  app never pays, because there `mc-core` is in-process. Rust spawn cost is reported
  separately, as information, not as part of TTFT.
- **Discipline.** 3 warmups + **12 measured** turns per cell, back-to-back, same
  machine, same 60x8-token text scenario for both brains (the mock ignores the prompt
  so both traverse identical work). Reported as **median (p10-p90)**. Two cells:
  **0 ms delay** (max-throughput: isolates framework parse/emit cost) and **20 ms
  delay** (realistic: ~50 chunks/s streaming, first-token latency injected).

## Results — text scenario (60 chunks x 8 tokens), median (p10-p90)

### Cell A — 0 ms per-chunk delay (framework overhead, no injected latency)

| Metric | Rust (rig) | TS (ai-sdk) |
|---|---|---|
| Server-anchored TTFT (ms) | **0.76** (0.71-0.80) | **0.87** (0.77-1.19) |
| Turn total (ms) | **2.26** (2.07-2.49) | **3.92** (2.99-4.87) |
| Throughput (chunks/s)¹ | ~40 000 | ~26 000 (17k-30k) |
| Peak RSS | **19.8 MB** | 191.0 MB (JS heap 49.8 MB) |

### Cell B — 20 ms per-chunk delay (realistic streaming)

| Metric | Rust (rig) | TS (ai-sdk) |
|---|---|---|
| Server-anchored TTFT (ms) | **22.6** (21.7-22.7) | **21.8** (21.3-22.9) |
| Turn total (ms) | **1315.8** (1302.6-1335.9) | **1306.3** (1300.2-1331.1) |
| Throughput (chunks/s) | ~46 | 46.8 (45.9-47.0) |
| Peak RSS | **21.3 MB** | 181.0 MB (JS heap 45.5 MB) |

¹ 0 ms throughput is measured over a sub-millisecond window and is noise-dominated;
read it only as "both drain the payload orders of magnitude faster than any network."
At 20 ms both converge on the injected ~50 chunks/s (~47 after overhead) — i.e. per-chunk
framework overhead is in the single-digit-percent-of-20 ms range for both, and equal
within noise.

### Supplementary (informational)

| Metric | Rust (rig) | TS (ai-sdk) |
|---|---|---|
| Process spawn + config load (ms)² | 19.9 (16.4-21.4) | n/a (in-process) |
| Whole CLI wall time, 0 ms cell (ms)² | 31.1 (26.5-31.8) | n/a |
| Client-anchored TTFT, 0 ms (ms)³ | n/a | 2.43 (1.90-3.02) |

² Rust CLI only. The desktop app embeds `mc-core` and does **not** spawn per turn,
so this is not a per-turn cost there — it matters only for the `mc` CLI itself.
³ TS in-process `runTurn` start -> first delta (what the webview actually sees, minus
IPC). It's ~1.6 ms above the server-anchored TTFT — that gap is the AI SDK's stream
setup + first-chunk parse.

## Tool round-trip: rig vs ai-sdk request counts

One tool-call scenario (mock returns a `tool_call`, then text after the tool result
comes back). **Both brains issued exactly 2 requests per turn** (`tool_call` -> `text`),
same shape, no extra/retry calls. No behavioral divergence observed in request count
for the happy path.

## Serde micro-benchmark (`serde-payload.mjs`)

`JSON.stringify` + `JSON.parse` of a **100.3 KB** SearchMods-shaped payload, x1000:
**0.127 ms per round-trip** (~770 MB/s). This approximates only the **webview-side
JSON share** of one Tauri IPC call. It does **not** measure the native Tauri transport;
crossing the webview<->Rust boundary adds roughly **~0.5-2 ms per call** per published
Tauri/IPC measurements (estimate, cited — not measured here). Note this cost is
largely **common to both brains**: the Rust brain's tools already run in `mc-core`,
and a TS brain running in the webview would call the *same* Tauri commands for tools,
paying the same IPC.

## Real-network smoke (TS core -> live OpenRouter, one turn, not looped)

| Model | TTFT | Total | Deltas | Tool calls |
|---|---|---|---|---|
| `deepseek/deepseek-v4-pro` | **1868.5 ms** | **2777.8 ms** | 43 | 0 |

End-to-end streaming through the TS brain works against real OpenRouter with no mock.
Reply came back correctly (in Chinese, the default). This is the reference scale for
everything above: **first token took ~1.9 s; the entire framework overhead measured in
the mock cells is ~2-4 ms, i.e. ~0.1-0.2% of it.**

## Caveats (what this does and does NOT measure)

- **Not measured: real Tauri IPC in the webview.** The desktop TS brain would forward
  tool calls over Tauri `invoke`; that transport (~0.5-2 ms/call estimate) is not
  exercised here. It applies per *tool call*, not per token, and is common to both.
- **RSS is not the in-app marginal cost.** The Node figure (~190 MB) includes the
  Node/V8 runtime **and `tsx`** (a dev-only TS transpiler that is absent in production —
  vite bundles ahead of time). The webview V8 is already resident in the shipped app,
  so the TS brain's marginal cost is closer to its **JS heap (~45-50 MB)**, and less
  than that after tree-shaking. The Rust ~20 MB is a whole standalone CLI process;
  in-app, `mc-core` shares the Tauri process, so its marginal cost is also smaller than
  the standalone number. **Neither RSS number is apples-to-apples for the desktop app;**
  treat them as loose upper bounds for two very different hosting models.
- **CLI spawn cost (~20 ms) is a mirage for the desktop path** — included in the CLI's
  wall time but excluded from the fair TTFT, because the app never spawns per turn.
- **Node JIT warmup** is handled by 3 warmups; the first cold turn was discarded.
- **Mock, not a model.** No real token generation, no server-side queueing, no TLS.
  The mock isolates client-side framework cost on purpose; the real-net smoke supplies
  the missing scale factor.
- **Debug Rust build.** The `mc` binary is `target/debug` (unoptimized). A release
  build would only *lower* the Rust numbers; the conclusion is unaffected.

## Conclusion

At the resolution that matters to a user, the two brains are the same. In the
zero-latency limit the TS brain adds about **1.6 ms** over Rust to run a full 60-chunk
turn (~3.9 ms vs ~2.3 ms), and its first-token latency is within a millisecond of
Rust's (~0.9 ms vs ~0.8 ms, server-anchored). The moment any real network is present
those differences vanish into the noise floor: a live OpenRouter turn spent **1869 ms
before the first token and 2778 ms in total**, so the entire framework overhead is on
the order of **0.1% of a real turn.** Streaming throughput, tool-round-trip request
counts, and per-chunk overhead are equal within measurement noise. The only real
axis of difference is memory hosting model (a fresh Rust process is ~20 MB; a Node/V8
host is heavier — but the shipped app already runs V8, and the marginal JS-heap cost
is ~45-50 MB, dev-tooling excluded), and that is a deployment-shape decision, not a
per-turn performance one. **Choose between the Rust and TS brains on architecture,
code-sharing, and maintainability — performance does not distinguish them.**

---

*Reproduce: `node bench/run-all.mjs` (writes `bench/results.json`), or the Rust half
alone via `bash bench/run-rust.sh`. Real-net smoke: `cd desktop && node --import tsx
bench/smoke-real.mjs` (needs `OPENROUTER_API_KEY`). Numbers above from
`bench/results.json`, captured 2026-07-01, Node v26, macOS/Darwin 24.1, debug `mc` build.*
