# @kobemc/agent-core

The streaming modpack-agent brain. It owns the prompt, AI SDK agent loop, and
tool schemas; it does not own launcher-side execution.

`agent-core` imports `ai`, `zod`, and provider/runtime packages only. It has no
React, Tauri, or launcher UI imports, so the same brain can run in the desktop
webview today and move to a hosted server later.

## Layout

```
src/
  agent.ts       createModpackAgent(settings) -> ModpackAgent
  harness/       Claude Code local-runtime engine, same ModpackAgent contract
  tools/         AI SDK client-tool schemas, one file per tool
  prompt.ts      BUILD_AGENT_SYSTEM_PROMPT / INSTANCE_AGENT_SYSTEM_PROMPT
  types.ts       shared schema/types
bin/
  mc-agent.mjs   headless debug CLI
  harness-host.mjs
test/            vitest coverage for the agent loop and schemas
```

## Boundary

Both engines expose the same launcher contract:

```ts
agent.run(history, onUpdate, signal) -> Promise<{ messages, error? }>
```

The UI only stores and renders AI SDK `UIMessage[]`. OpenRouter
`ToolLoopAgent` and the Claude Code `HarnessAgent` differ only in who owns the
model loop:

- `createModpackAgent(settings)` runs the OpenRouter/OpenAI-compatible API path.
- `createClaudeCodeModpackAgent(handlers, options?)` runs the local Claude Code
  subscription runtime through a Node harness host.

## Profiles And Tools

The launcher selects one explicit profile before a conversation starts. Each
profile injects its own prompt and complete relevant tool set; the model chooses
among those tools normally, without a separate activation step.

- `build`: global modpack discovery, exact plan validation, build, and install-card flow.
- `instance`: one host-bound installed instance, combining local wiki, diagnosis,
  compatible mod discovery, and user-confirmed maintenance changes.

All modpack tools are AI SDK client-side tools: they have schemas, descriptions,
and no `execute` in the OpenRouter/webview path.

When the model emits a tool part in `input-available`, the launcher client runs
the tool through its existing Rust IPC commands, writes the structured result
back to that `UIMessage` as `output-available`, then calls `run()` again with
the same history.

That keeps the Rust daemon as the single source of truth for Modrinth search,
resolution, build, install, and local instance state:

- `search_base_modpacks`
- `inspect_base_modpack`
- `search_mods`
- `mod_get_detail`
- `resolve_mods`
- `validate_modpack_plan`
- `confirm_modpack_build` (the launcher executes the build only after the user confirms the card)
- `list_instances`
- `diagnose_instance`
- `confirm_deep_diagnosis`
- `run_diagnostic_trial`
- `finish_deep_diagnosis`
- `ask_user_question`
- `show_modpack`
- `show_instance_changes`

The desktop dispatcher lives in `desktop/src/agent/clientToolDispatcher.ts`.
Interactive tools (`ask_user_question`, `confirm_modpack_build`,
`confirm_deep_diagnosis`, `show_modpack`, `show_instance_changes`) are resolved by UI components; automatic tools are resolved by IPC commands such as
`agent_tool_search_mods`.

The local Claude Code runtime cannot call Tauri IPC directly, so
`harness-host.mjs` attaches bridge handlers that proxy tool calls over stdio
back to the webview. Those handlers are transport glue, not a second tool
implementation.

## CLI

```
mc-agent chat "<prompt>" [--engine openrouter|claude-code] [--model X] [--json] [--turns "a||b"]
```

The CLI is a headless stream/debug harness. It prints text and tool calls, but
does not execute launcher client tools because it has no Rust IPC/UI client.
Endpoint settings come from `OPENROUTER_API_KEY`, `OPENROUTER_MODEL`, and
`OPENROUTER_BASE_URL`; a repo-root `.env` is loaded when present.

```
node packages/agent-core/bin/mc-agent.mjs chat "a chill 1.20.1 fabric pack"
```

## Deterministic Eval

The lightweight profile gate checks tool boundaries, prompt routing, host-owned
field isolation, and the absence of progressive tool activation. It does not
call an external model:

```sh
npm run eval:instance --workspace @kobemc/agent-core -- --json
```
