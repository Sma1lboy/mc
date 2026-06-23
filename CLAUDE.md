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

- **Tauri commands are thin.** No launcher logic in `desktop/src-tauri/src/commands.rs` —
  it maps a UI call to an `mc-core` call and serialises the result. Logic lives in `mc-core`.
- **UI state** is module-level SolidJS signals in `desktop/src/store.ts` (no Context/Router).
  Pages import and read/write signals directly. IPC is funnelled through `desktop/src/ipc/api.ts`.
- **Two UI layouts** coexist, switched by `layoutMode`: `modrinth` (dark) and `pcl` (light,
  faithful to PCL2). See `desktop/src/layout/`.
- **Auth** funnels all account kinds (offline / Microsoft / Yggdrasil) into one
  `AuthSession` (`mc-core/src/auth/`). Microsoft uses the device-code flow.

## Secrets / env (never commit real values)

- `desktop/src-tauri/.env` — `MC_MSA_CLIENT_ID` (your Azure app's public client id for
  Microsoft login). Copy from `.env.example`. The default vanilla id is rejected by the
  device-code endpoint (AADSTS700016); register your own Azure app.
- `crates/mc-server/.env` — `DATABASE_URL` (Postgres/Supabase). Local dev only.
- Both are gitignored. Only `*.env.example` is tracked.

## Don't

- Don't put launcher logic in the Tauri layer.
- Don't commit `.env`, build artifacts (`target/`, `dist/`, `node_modules/`), or `ref/`.
- Don't add AI/Claude attribution to commits or PRs.
