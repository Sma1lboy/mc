# @kobemc/agent-core (host-agnostic modpack-agent brain)

The streaming tool-use "brain" that turns a chat request into a verified
Minecraft `.mrpack`. It imports only `ai`, `zod`, and std TS — **no** react /
@tauri-apps / project UI — so the same code runs in the desktop webview, a Node
CLI, or a hosted server. Host-specific bits are injected as a `ToolExecutor`.

## Layout

```
src/
  agent.ts       createModpackAgent(settings, tools) → { runTurn }
  tools.ts       the six zod tool schemas (mirror the Rust *Args), buildTools()
  prompt.ts      CHAT_AGENT_SYSTEM_PROMPT (ported verbatim from Rust)
  types.ts       AgentStreamEvent / ChatMessage / ToolExecutor / AgentLlmSettings
  executors/     host tool backends (below)
bin/mc-agent.mjs the test CLI (see below)
test/            vitest: brain (mock LLM) + modrinthExecutor (mocked HTTP)
```

## Boundary

`createModpackAgent(settings, tools)` → `{ runTurn(history, userMessage, onEvent) }`.
`settings: AgentLlmSettings` picks the LLM endpoint (any OpenAI-compatible base
URL); `tools: ToolExecutor` is a `{ [toolName]: (args) => Promise<output> }` map
the host injects — the core never calls Tauri/HTTP itself. `runTurn` emits the
same snake-case `AgentStreamEvent` union as the Rust brain (`text_delta` /
`reasoning` / `tool_call` / `tool_result` / `done` / `error`), so one UI reducer
serves both. The returned `history` seeds the next turn.

## Executors (`@kobemc/agent-core/executors`)

- **`mockExecutor(fixtures?)`** — instant canned outputs (tests / offline demos);
  pass `fixtures` to override individual tools (e.g. a spy).
- **`modrinthExecutor(opts?)`** — a REAL, read-only backend over the Modrinth
  HTTP API for the five non-writing tools (search / inspect / detail / resolve),
  with output field names matching the Rust tools exactly. It never writes to
  disk: `build_modpack` returns a structured error (`{ status: "unsupported",
  error: "building is not available in this host — use the desktop app" }`).

The desktop host does NOT use these — it binds the six tool names to Tauri
`invoke` (`desktop/src/agent/desktopAdapter.ts`), keeping `@tauri-apps` out of
this package.

## Hosting on a server

Wrap `runTurn` in an SSE/POST route: create the agent from the request's
settings + `modrinthExecutor()`, then forward each `AgentStreamEvent` as one SSE
`data:` line. The returned `history` is the transcript to persist per session.

**Building on a server is still TBD.** `build_modpack` is the only trust-critical,
disk-writing tool (it re-resolves every file through the provider and assembles a
`.mrpack`); it is deliberately NOT reimplemented in TS. A server that needs to
build should delegate to the Rust executor (`mc_core::agent::chat::tools::
tool_build_modpack`) or carefully port it — not extend `modrinthExecutor`.

## CLI (`mc-agent`)

```
mc-agent chat "<prompt>" [--executor mock|modrinth] [--model X] [--json] [--turns "a||b"]
```

Streams TextDelta live and prints tool chips (`🔧 name(args)` / `✓ name: summary`);
`--json` prints one event per line. Endpoint from the env (`OPENROUTER_API_KEY`,
`OPENROUTER_MODEL`, `OPENROUTER_BASE_URL`); a repo-root `.env` is loaded when
present. Runs under plain `node` (registers tsx to load the TS source):

```
node packages/agent-core/bin/mc-agent.mjs chat "a chill 1.20.1 fabric pack"
```
