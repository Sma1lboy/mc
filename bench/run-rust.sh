#!/usr/bin/env bash
# Standalone Rust-brain (rig) cell: start the mock, run K warmup + N measured
# `mc agent chat` turns (timed via run-rust.mjs on a clock shared with the mock),
# then one /usr/bin/time -l invocation for peak RSS. Prints JSON, tears down.
#
# The full both-brains comparison is `node bench/run-all.mjs`; this script is the
# Rust half in isolation. Env overrides: PORT WARMUP MEASURED CHUNKS TOKENS_PER_CHUNK DELAY_MS.
set -euo pipefail

ROOT="/Users/jacksonc/i/mc-launcher/.claude/worktrees/agent-ts-brain"
BENCH="$ROOT/bench"
MC_BIN="$ROOT/target/debug/mc"
PORT="${PORT:-8799}"
WARMUP="${WARMUP:-3}"
MEASURED="${MEASURED:-12}"
CHUNKS="${CHUNKS:-60}"
TOKENS_PER_CHUNK="${TOKENS_PER_CHUNK:-8}"
DELAY_MS="${DELAY_MS:-0}"
PROMPT="Recommend a good 1.20.1 Fabric performance modpack."

# Start mock.
PORT="$PORT" SCENARIO=text CHUNKS="$CHUNKS" TOKENS_PER_CHUNK="$TOKENS_PER_CHUNK" DELAY_MS="$DELAY_MS" \
  node "$BENCH/mock-openrouter.mjs" >/tmp/mock-rust-cell.out 2>&1 &
MOCK_PID=$!
trap 'kill "$MOCK_PID" 2>/dev/null || true' EXIT
# Wait for ready line.
for _ in $(seq 1 50); do grep -q '"ready":true' /tmp/mock-rust-cell.out 2>/dev/null && break; sleep 0.1; done

echo "== timed loop (server-anchored TTFT + total, spawn overhead) =="
PORT="$PORT" WARMUP="$WARMUP" MEASURED="$MEASURED" MC_BIN="$MC_BIN" CWD="$ROOT" \
  node "$BENCH/run-rust.mjs"

echo "== peak RSS (one invocation, /usr/bin/time -l) =="
OPENROUTER_BASE_URL="http://127.0.0.1:$PORT/v1" OPENROUTER_API_KEY=bench MC_AGENT_OPENROUTER_MODEL=bench MC_LOG=error \
  /usr/bin/time -l "$MC_BIN" agent chat "$PROMPT" >/dev/null 2>/tmp/rust-rss.out || true
grep 'maximum resident set size' /tmp/rust-rss.out || echo "(RSS line not found)"
