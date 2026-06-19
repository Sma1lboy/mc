---
description: Build & (re)start the mc-launcher desktop app for testing (vite + Tauri binary)
argument-hint: "[ui] [server] — ui = skip Rust rebuild (frontend HMR only); server = also start mc-server :8787"
---

Start (or restart) the mc-launcher desktop app for testing by running the dev script, then report status.

Run this exact command (resolves the repo root portably):

```bash
bash "$(git rev-parse --show-toplevel)/scripts/dev-app.sh" $ARGUMENTS
```

The script ensures the vite dev server is up on :1420, rebuilds the debug Tauri binary, stops the old app process, and relaunches `target/debug/mc-launcher-desktop`.

After it runs:
- If it prints a build failure or "app did not stay running", read the log tail it printed and surface the actual error to me. Do **not** retry blindly or loop.
- On success, confirm in one line: app pid, binary build time, and the log paths (`/tmp/mc-desktop.log`, `/tmp/mc-vite.log`).
- Do not try to screenshot the app — the nohup-launched window won't reliably raise above other windows; just tell me it's up and I'll look.

When to use which:
- **Frontend-only change** (`.tsx` / `.css` / `.ts` under `desktop/src`): use `/run-app ui` — skips the Rust rebuild, vite HMR applies it. Much faster.
- **Rust / Tauri change** (new `#[tauri::command]`, `mc-core`, `commands.rs`, `lib.rs`): use `/run-app` — a full rebuild is required, otherwise a new `invoke()` call hits a binary that doesn't have the command.
- **Testing email/password (better-auth) login**: add `server` to also bring up the mc-server backend.
