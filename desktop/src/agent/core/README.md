# modpack-agent core (host-agnostic brain)

`agent/core/` imports only `ai`, `zod`, and std TS — **no** react / @tauri-apps /
project UI. So it runs unchanged in the desktop webview, the daemon, or a hosted
mc-server. Host-specific bits live in adapters.

- **Boundary.** `createModpackAgent(settings, tools)` → `{ runTurn(history, userMessage, onEvent) }`.
  `settings: AgentLlmSettings` picks the LLM endpoint (any OpenAI-compatible base
  URL). `tools: ToolExecutor` is a `{ [toolName]: (args) => Promise<output> }` map
  the host injects — the core never calls Tauri/HTTP itself.
- **Adapter injects tools.** Desktop binds the six names to Tauri `invoke`
  (`agent/desktopAdapter.ts`); the daemon would bind them to in-process mc-core
  calls; mc-server to its own resolver.
- **Hosting on mc-server.** Wrap `runTurn` in an SSE/POST route: create the agent
  from the request's settings + a server-side `ToolExecutor`, then forward each
  `AgentStreamEvent` as one SSE `data:` line (the tags already match the Rust wire
  format, so the same UI reducer consumes it). The returned `history` is the
  transcript to persist per session and pass back on the next turn.
- **Events.** `runTurn` emits the same snake-case `AgentStreamEvent` union as Rust
  (`text_delta` / `reasoning` / `tool_call` / `tool_result` / `done` / `error`),
  so one reducer serves both brains.
