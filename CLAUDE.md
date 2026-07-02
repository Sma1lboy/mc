# CLAUDE.md

Guidance for Claude Code (and contributors using it) when working in this repo.

## What this is

A from-scratch, cross-platform Minecraft launcher. Rust core does all the
launcher logic; a thin Tauri shell + SolidJS UI render it; an optional axum
backend (`mc-server`) provides account/email-password auth.

**Stack:** Rust core (`mc-core`) + Tauri v2 + SolidJS + axum (`mc-server`).

## Layout

```
crates/
  mc-types    shared DTOs (AuthSession, AccountSummary, InstanceSummary, …)
  mc-core     all launcher logic: version/, download/, java/, auth/, launch/,
              loader/, instance/, modplatform/, meta, diagnostics, settings, server
  mc-cli      headless CLI over mc-core
  mc-server   axum + better-auth backend (email/password), Postgres via sqlx
desktop/
  src/        SolidJS UI (pages/, layout/, components/, ipc/, store.ts, theme/)
  src-tauri/  Tauri shell — commands.rs is a THIN glue layer over mc-core
scripts/      dev-app.sh — build & (re)start the desktop app for testing
docs/         engineering reference + design notes
ref/          reference launchers for study (PrismLauncher/PCL2/PCL-CE) — NOT
              part of this project; gitignored, present only on local machines
```

## Build / run / test

```bash
# Rust (from repo root)
cargo build
cargo test                       # full suite

# Desktop app (dev) — easiest:
scripts/dev-app.sh               # full: rebuild Rust binary + restart app
scripts/dev-app.sh ui            # frontend-only: skip Rust build, vite HMR
scripts/dev-app.sh server        # also start mc-server (:8787)
# In Claude Code, the same is available as /run-app [ui|server].
```

- The debug Tauri binary loads its UI from the vite dev server on **:1420**;
  `dev-app.sh` ensures it's up. `mc-server` runs on **:8787**.
- Frontend-only change → `ui` mode (HMR). Rust / new `#[tauri::command]` → full
  rebuild (a new `invoke()` won't exist in an old binary).

## Logs (debugging)

One unified, daily-rolling log at **`<data_dir>/logs/mc-launcher.log`** (Settings → 诊断 →
「打开日志目录」, or the `open_logs_dir` command). Set up in `desktop/src-tauri/src/logging.rs`;
captures both sides, distinguished by the `tracing` **target**:
- **daemon** (local data layer) — `mc-core` + command-layer `tracing` events; target is the Rust
  module path (e.g. `mc_core::launch`). Use `tracing::{info,warn,…}!` in Rust.
- **client** (webview) — forwarded via the `client_log` command (target `client:`); the frontend
  `desktop/src/util/log.ts` mirrors `console.error`/`warn` + `window.onerror` there. Use `log.*`.

`MC_LOG` (or `RUST_LOG`) overrides the filter, e.g. `MC_LOG=mc_core=trace`. Debug builds also
mirror to stderr.

## Conventions

- **Check the library docs (context7) BEFORE building against any dependency.** This repo
  has the **context7** MCP server configured (`.mcp.json`). Before you integrate, extend, or
  hand-write anything that touches a third-party library/framework (the AI SDK `ai`,
  `@ai-sdk/*`, tauri, zod, solid/react, …), first pull its authoritative, version-matched
  docs: `resolve-library-id` → `get-library-docs`. Do NOT guess an API from memory or
  reinvent what the library already provides. Real lesson: the modpack agent hand-rolled a
  tool wrapper, a fake client-tool ack, and a whole streaming-event union + reducer —
  all of which the AI SDK already ships (`tool()`, client-side tools, `UIMessageChunk` /
  `readUIMessageStream` with `ToolUIPart.state`). Look it up, then reuse.
- **Dev loop: Ladle for UI, CLI for the core — one dataflow.** Iterate on UI in **Ladle**
  (`npm run ladle -w desktop` → :61000): render/tweak components as `*.stories.tsx`, don't
  launch the whole Tauri app to eyeball a component (see the `ladle-ui` skill; edit → HMR →
  screenshot loop). Debug the backend/agent **headless via the CLIs**: `mc-agent`
  (`packages/agent-core/bin` — runs the TS brain + tools/executors against mock/modrinth,
  `--json`) and `mc` (`crates/mc-cli` — launcher core). Both ends drive the **same dataflow**:
  agent-core (brain + tools) ↔ the AI SDK `UIMessage` contract ↔ the UI. Ladle renders that
  contract with mock messages; the CLI drives the logic behind it — so a shared-core change is
  verifiable from either side without the full app.
- **Tauri commands are thin.** No launcher logic in `desktop/src-tauri/src/commands.rs` —
  it maps a UI call to an `mc-core` call and serialises the result. Logic lives in `mc-core`.
- **UI state** is module-level SolidJS signals in `desktop/src/store.ts` (no Context/Router).
  Pages import and read/write signals directly. IPC is funnelled through `desktop/src/ipc/api.ts`.
- **Two UI layouts** coexist, switched by `layoutMode`: `modrinth` (dark) and `pcl` (light,
  faithful to PCL2). See `desktop/src/layout/`.
- **Any user-facing string change MUST sync i18n.** No hardcoded display text — every string goes
  through `t("ns.key")`. When you add/edit/remove a string: add the key to the matching slice in
  `desktop/src/locales/` for **both** `zh` and `en` (zh is the source of truth; en falls back to zh
  but add a real translation), and delete keys you orphan. Use getter-scoped `t()` (never a
  module-level const holding `t()` — it freezes the language). Run `node scripts/check-i18n.mjs`
  (from `desktop/`) before committing; CI gates it. Default language is Chinese.
- **Don't reinvent type concepts — reuse the single source.** Before hand-writing a
  type or wrapper, check whether one already exists and derive from it. Use the
  framework's own primitives directly (e.g. AI SDK's `tool({ description, inputSchema,
  execute })` — don't wrap it in a bespoke tool abstraction). Derive types from their
  schema instead of maintaining a parallel `interface` (`type T = z.infer<typeof
  schema>`; Rust DTOs → generated `bindings.ts`, imported, never re-declared). One
  shape = one definition; a deliberately *different* shape (e.g. a normalized render
  model vs the wire type) is fine, a duplicated *identical* one is not.
- **Agent (modpack assistant) — search the framework before you build.** The brain is
  `@kobemc/agent-core` (host-agnostic TS: the loop, prompt, and tool *definitions*), built
  on the Vercel AI SDK (`ai` + `@openrouter/ai-sdk-provider`). The tool *executor* is
  injected per host — desktop binds each tool to Rust via the thin `agent_tool_*` Tauri
  commands (trust code — hash / path-sanitize / disk writes — stays in `mc-core`); a pure-TS
  Modrinth executor exists for non-desktop hosting. BEFORE adding or changing a tool or an
  interaction pattern, **search the SDK's own API/types first** (`node_modules/ai`,
  `@ai-sdk/provider-utils`) and reuse its native primitives instead of hand-rolling one:
  `tool({ inputSchema, execute })` — one self-contained file per tool under
  `packages/agent-core/src/tools/`, with `execute` co-located (host tools forward to the
  injected `exec`); a **client-side tool** is a tool with NO `execute` that pauses the turn
  and resumes via a tool-result (that's how `ask_user_question` works — do NOT fake it with
  an ack + a synthetic user message); derive arg types with `z.infer`. The stream event
  contract lives once in `mc-types::AgentStreamEvent` → generated `bindings.ts`.
- **Auth** funnels all account kinds (offline / Microsoft / Yggdrasil) into one
  `AuthSession` (`mc-core/src/auth/`). Microsoft uses the device-code flow.
- **Commits** follow Conventional Commits: `type(scope): subject` (lowercase subject),
  e.g. `feat(logging): …`, `fix(auth): …`, `docs: …`, `refactor: …`, `chore: …`.
- **Releases default to a PATCH bump** (`scripts/release.sh`): next version = `patch+1` of the
  latest `v*` tag (e.g. `0.1.0` → `0.1.1`), regardless of how many features landed. Only do a
  minor/major bump when the user **explicitly** says so. State the version before tagging.

## Working style (for AI agents in this repo)

Distilled from Anthropic's Claude Fable 5 prompting guide — the through-line is a goal
plus boundaries, not step lists:

- **Act when you have enough information.** Don't re-derive established facts or survey
  options you won't pursue; when weighing a choice, give one recommendation.
- **No unrequested scope.** A bug fix doesn't need surrounding cleanup. No abstractions,
  error handling, or flags beyond what the task requires; validate at system boundaries
  (user input, external APIs) only — trust internal code.
- **Ground progress claims in tool results from this session.** If tests fail, say so with
  the output; if a step was skipped, say that. Never report unverified work as done.
- **A problem description is not a change request.** When the user describes a problem or
  asks a question, the deliverable is your assessment — don't apply a fix until asked.
- **Lead with the outcome.** The first sentence of a summary says what happened or what you
  found; detail after, in complete sentences (no arrow-chain shorthand or invented labels).
- **Delegate independent subtasks to subagents** and keep working while they run; for long
  work, prefer fresh-context verifier subagents over self-review.
- **Pause only where genuinely needed:** a destructive or irreversible action, a real scope
  change, or input only the user can provide. Otherwise finish the work, then offer follow-ups.

## Secrets / env (never commit real values)

- `desktop/src-tauri/.env` — `MC_MSA_CLIENT_ID` (your Azure app's public client id for
  Microsoft login). Copy from `.env.example`. The default vanilla id is rejected by the
  device-code endpoint (AADSTS700016); register your own Azure app.
- `.env` at the repository root — `OPENROUTER_API_KEY` for the local AI agent. Copy
  from root `.env.example`. Optional: `MC_AGENT_OPENROUTER_MODEL`,
  `OPENROUTER_MODEL`, `OPENROUTER_BASE_URL`.
- `crates/mc-server/.env` — `DATABASE_URL` (Postgres/Supabase). Local dev only.
- These `.env` files are gitignored. Only `.env.example` templates are tracked.

## Don't

- Don't put launcher logic in the Tauri layer.
- Don't commit `.env`, build artifacts (`target/`, `dist/`, `node_modules/`), or `ref/`.
- Don't add AI/Claude attribution to commits or PRs.
