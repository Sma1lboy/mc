#!/usr/bin/env bash
# dev-app.sh — build & (re)start the mc-launcher desktop app for testing.
#
# What it does, in order:
#   1. Ensure the vite dev server is listening on :1420 for browser preview.
#   2. (optional, `server` arg) Ensure the mc-server auth backend is on :8787.
#   3. Rebuild the frontend bundle used by the directly launched debug binary.
#   4. Rebuild the debug Tauri binary (skip with `ui` for frontend-only work).
#   5. Stop the old desktop app process and relaunch the fresh binary.
#   6. Best-effort bring the window to the front, then print status + log paths.
#
# Usage:
#   scripts/dev-app.sh            # full: rebuild Rust + restart app
#   scripts/dev-app.sh ui         # frontend-only: skip cargo build (fast)
#   scripts/dev-app.sh server     # also start mc-server (:8787) for email/pw auth
#   scripts/dev-app.sh ui server  # combine flags
set -uo pipefail

# Repo root, derived from this script's own location (portable across machines).
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESKTOP="$ROOT/desktop"
TAURI="$DESKTOP/src-tauri"
BIN="$TAURI/target/debug/mc-launcher-desktop"
VITE_LOG="/tmp/mc-vite.log"
APP_LOG="/tmp/mc-desktop.log"
SRV_LOG="/tmp/mc-server.log"

SKIP_BUILD=0
WITH_SERVER=0
for a in "$@"; do
  case "$a" in
    ui|nobuild|fast) SKIP_BUILD=1 ;;
    server|auth)     WITH_SERVER=1 ;;
    help|-h|--help)  sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "⚠ unknown arg: $a (use: ui | server | help)" ;;
  esac
done

# Make cargo available even in a fresh non-login shell.
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

# Load local dev env (MC_MSA_CLIENT_ID for Microsoft login, MC_SERVER_URL, …).
# Gitignored; copy src-tauri/.env.example to src-tauri/.env and fill it in.
if [ -f "$TAURI/.env" ]; then
  echo "✓ loading env from src-tauri/.env"
  set -a; source "$TAURI/.env"; set +a
fi

# 1) vite dev server on :1420 -------------------------------------------------
if lsof -nP -iTCP:1420 -sTCP:LISTEN >/dev/null 2>&1; then
  echo "✓ vite already serving on :1420"
else
  echo "→ starting vite dev server…"
  ( cd "$DESKTOP" && nohup npm run dev > "$VITE_LOG" 2>&1 & )
  for _ in $(seq 1 40); do
    sleep 0.5
    curl -sf -o /dev/null http://localhost:1420/ && break
  done
  if curl -sf -o /dev/null http://localhost:1420/; then
    echo "✓ vite up on :1420"
  else
    echo "✗ vite did not come up — see $VITE_LOG"; tail -15 "$VITE_LOG" 2>/dev/null
  fi
fi

# 2) optional mc-server on :8787 ---------------------------------------------
if [ "$WITH_SERVER" = 1 ]; then
  if lsof -nP -iTCP:8787 -sTCP:LISTEN >/dev/null 2>&1; then
    echo "✓ mc-server already on :8787"
  else
    echo "→ starting mc-server (background; first run compiles)…"
    ( cd "$ROOT/crates/mc-server" && nohup cargo run > "$SRV_LOG" 2>&1 & )
    echo "  log: $SRV_LOG"
  fi
fi

# 3) build the frontend bundle ------------------------------------------------
# Directly running target/debug/mc-launcher-desktop loads frontendDist, not the
# full `tauri dev` harness. Refresh dist so UI-only iterations are not stale.
echo "→ npm build (frontend bundle)…"
if ! ( cd "$DESKTOP" && npm run build ); then
  echo "✗ frontend build failed — leaving the running app untouched"; exit 1
fi

# 4) build the debug binary ---------------------------------------------------
if [ "$SKIP_BUILD" = 1 ]; then
  echo "↷ skipping cargo build (ui mode — frontend bundle refreshed)"
else
  echo "→ cargo build (desktop shell)…"
  if ! ( cd "$TAURI" && cargo build ); then
    echo "✗ build failed — leaving the running app untouched"; exit 1
  fi
fi

if [ ! -x "$BIN" ]; then
  echo "✗ binary not found: $BIN  (run without 'ui' to build it first)"; exit 1
fi

# 5) restart the desktop app --------------------------------------------------
OLD=$(pgrep -f mc-launcher-desktop)
if [ -n "$OLD" ]; then
  echo "→ stopping old app (pid $(echo "$OLD" | tr '\n' ' '))"
  kill $OLD 2>/dev/null
  sleep 1
fi
echo "→ launching fresh binary…"
# Launch from the repo root so repository-level dev config such as `.env`
# resolves consistently even when this script is invoked from another cwd.
( cd "$ROOT" && nohup "$BIN" > "$APP_LOG" 2>&1 & )
sleep 3
NEW=$(pgrep -f mc-launcher-desktop | head -1)

# 5) bring to front (best effort; nohup windows don't always raise) -----------
osascript -e 'tell application "System Events" to tell process "mc-launcher-desktop" to set frontmost to true' 2>/dev/null || true

echo ""
if [ -n "$NEW" ]; then
  echo "✓ app running — pid $NEW"
  echo "  binary built: $(date -r "$BIN" '+%Y-%m-%d %H:%M:%S')"
  echo "  app log:  $APP_LOG"
  echo "  vite log: $VITE_LOG"
  [ "$WITH_SERVER" = 1 ] && echo "  srv log:  $SRV_LOG"
  if grep -iqE 'error|panic' "$APP_LOG" 2>/dev/null; then
    echo "⚠ found error/panic in app log:"; grep -iE 'error|panic' "$APP_LOG" | tail -8
  fi
else
  echo "✗ app did not stay running — last lines of $APP_LOG:"
  tail -20 "$APP_LOG" 2>/dev/null
  exit 1
fi
