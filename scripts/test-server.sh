#!/usr/bin/env bash
# test-server.sh — run mc-server's integration tests (realm lifecycle +
# permissions, share roundtrip) against a real Postgres. This is the "本地 sync
# 测试": it exercises the real SQL (realms / realm_members / accounts) end to end.
#
# Use a **Supabase free-tier dev** project (recommended) or any Postgres:
#   1) put the dev connection string in crates/mc-server/.env:
#        DATABASE_URL=postgresql://...:...@...pooler.supabase.com:5432/postgres
#      (Supabase → Project → Connect → Session pooler URI), or
#      export TEST_DATABASE_URL=... in your shell.
#   2) scripts/test-server.sh
#
# The tests are self-cleaning (fixed `t-realm-*` ids, deleted on completion), so
# they're safe against a dev DB. Optional local Docker Postgres instead:
#   cd crates/mc-server && docker compose up -d --wait
#   DATABASE_URL=postgres://mc:mc@127.0.0.1:55432/mc_server scripts/test-server.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

# Fall back to crates/mc-server/.env for DATABASE_URL when not already exported.
ENVF="$ROOT/crates/mc-server/.env"
if [ -z "${DATABASE_URL:-}" ] && [ -f "$ENVF" ]; then
  set -a
  # shellcheck disable=SC1090
  . "$ENVF"
  set +a
fi

URL="${TEST_DATABASE_URL:-${DATABASE_URL:-}}"
if [ -z "$URL" ]; then
  echo "✗ no database url."
  echo "  Put a Supabase free-tier dev URI in $ENVF (DATABASE_URL=...),"
  echo "  or export TEST_DATABASE_URL=...  (see this script's header for details)."
  exit 1
fi

# Don't print the full URL (it has a password); just the host for confirmation.
host=$(printf '%s' "$URL" | sed -E 's#^[a-z]+://[^@]*@?([^:/]+).*#\1#')
echo "→ running mc-server integration tests against ${host:-the configured dev Postgres}…"
TEST_DATABASE_URL="$URL" cargo test -p mc-server
code=$?
[ "$code" = "0" ] && echo "✓ 本地 sync 测试 passed" || echo "✗ tests failed (exit $code)"
exit "$code"
