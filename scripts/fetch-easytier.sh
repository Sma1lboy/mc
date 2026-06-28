#!/usr/bin/env bash
#
# fetch-easytier.sh — download the EasyTier binaries for THIS build host and place
# them where the Tauri bundle picks them up (`desktop/src-tauri/resources/easytier/`).
#
# The launcher's binary resolver looks for `easytier-core` / `easytier-cli` next to
# the app (and, on macOS, in `…/Contents/Resources/easytier/`). The Tauri config
# ships `resources/easytier/*` into the app's resource dir under `easytier/`, so the
# binaries we drop here end up bundled inside the .app — users never install EasyTier.
#
# Run before `tauri build` (the release flow does this; see scripts/release.sh).
# Idempotent: if the binaries already exist and match $EASYTIER_VERSION, it skips the
# download. Only fetches the binaries for the host platform/arch.
#
#   scripts/fetch-easytier.sh            # fetch for this host
#   EASYTIER_VERSION=v2.6.4 scripts/fetch-easytier.sh
#
set -euo pipefail

EASYTIER_VERSION="${EASYTIER_VERSION:-v2.6.4}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="$ROOT/desktop/src-tauri/resources/easytier"

# --- Detect host platform/arch → EasyTier release asset name ---------------------
uname_s="$(uname -s)"
uname_m="$(uname -m)"
EXE=""
case "$uname_s" in
  Darwin)
    case "$uname_m" in
      arm64|aarch64) asset="easytier-macos-aarch64-${EASYTIER_VERSION}" ;;
      x86_64)        asset="easytier-macos-x86_64-${EASYTIER_VERSION}" ;;
      *) echo "error: unsupported macOS arch '$uname_m'" >&2; exit 2 ;;
    esac
    ;;
  Linux)
    case "$uname_m" in
      x86_64) asset="easytier-linux-x86_64-${EASYTIER_VERSION}" ;;
      *) echo "error: unsupported Linux arch '$uname_m'" >&2; exit 2 ;;
    esac
    ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    asset="easytier-windows-x86_64-${EASYTIER_VERSION}"
    EXE=".exe"
    ;;
  *)
    echo "error: unsupported OS '$uname_s'" >&2; exit 2 ;;
esac

CORE="$DEST/easytier-core${EXE}"
CLI="$DEST/easytier-cli${EXE}"
STAMP="$DEST/.version"

# --- Idempotent: skip if already present and the recorded version matches --------
if [ -f "$CORE" ] && [ -f "$CLI" ] && [ -f "$STAMP" ] \
   && [ "$(cat "$STAMP" 2>/dev/null)" = "$EASYTIER_VERSION" ]; then
  echo "==> EasyTier ${EASYTIER_VERSION} already present in resources/easytier/ — skipping"
  echo "    $CORE"
  echo "    $CLI"
  exit 0
fi

url="https://github.com/EasyTier/EasyTier/releases/download/${EASYTIER_VERSION}/${asset}.zip"
echo "==> Fetching EasyTier ${EASYTIER_VERSION} for ${uname_s}/${uname_m}"
echo "    $url"

mkdir -p "$DEST"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

zip="$tmp/easytier.zip"
curl -fSL "$url" -o "$zip"
unzip -q -o "$zip" -d "$tmp"

# The archive contains a folder ("$asset/") with easytier-core / easytier-cli inside.
src_core="$(find "$tmp" -type f -name "easytier-core${EXE}" -print -quit)"
src_cli="$(find "$tmp" -type f -name "easytier-cli${EXE}" -print -quit)"
if [ -z "$src_core" ] || [ -z "$src_cli" ]; then
  echo "error: easytier-core/easytier-cli not found inside $asset.zip" >&2
  exit 1
fi

cp "$src_core" "$CORE"
cp "$src_cli" "$CLI"
chmod +x "$CORE" "$CLI"

# macOS: strip the quarantine xattr so the bundled binaries run without a Gatekeeper prompt.
if [ "$uname_s" = "Darwin" ]; then
  xattr -dr com.apple.quarantine "$DEST" 2>/dev/null || true
fi

printf '%s' "$EASYTIER_VERSION" > "$STAMP"

echo "==> Placed EasyTier ${EASYTIER_VERSION}:"
echo "    $CORE"
echo "    $CLI"
