#!/usr/bin/env bash
# Fetch the pinned InterMed static analyzer for a Tauri build target.
set -euo pipefail

INTERMED_VERSION="v0.1.7-alpha"
INTERMED_COMMIT="aa1df9a215ba4c7bb26a37da48db3b81f87539a5"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST_DIR="$REPO_ROOT/desktop/src-tauri/resources/intermed"
TARGET="${1:-$(rustc --print host-tuple)}"

case "$TARGET" in
  aarch64-apple-darwin)
    ASSET="intermed-${INTERMED_VERSION}-macos-aarch64.tar.gz"
    EXPECTED_SHA="aa9e2ab6d48ee4826199fb668cb739364759f4726c4cebbac1b0b8075e0e8c10"
    EXE=""
    ;;
  x86_64-unknown-linux-gnu)
    ASSET="intermed-${INTERMED_VERSION}-linux-x86_64.tar.gz"
    EXPECTED_SHA="af5372050d596842d01e5bb87ca86eb0b6ef99eebcfebd092d5be64dd40556e1"
    EXE=""
    ;;
  x86_64-pc-windows-msvc)
    ASSET="intermed-${INTERMED_VERSION}-windows-x86_64.zip"
    EXPECTED_SHA="f93bf3fb636758a1928129cf6d168fb4eee1588639b621f97c8ac5c629423bd1"
    EXE=".exe"
    ;;
  x86_64-apple-darwin)
    # Upstream does not publish an Intel macOS archive. Build the exact tagged
    # commit with the release toolchain instead of silently dropping support.
    ASSET=""
    EXPECTED_SHA=""
    EXE=""
    ;;
  *)
    echo "error: unsupported InterMed build target '$TARGET'" >&2
    exit 2
    ;;
esac

DEST_BIN="$DEST_DIR/intermed${EXE}"
STAMP="$DEST_DIR/.version"
if [ -x "$DEST_BIN" ] && [ -f "$STAMP" ] \
  && [ "$(sed -n '1p' "$STAMP")" = "$INTERMED_VERSION@$INTERMED_COMMIT:$TARGET" ]; then
  echo "==> InterMed $INTERMED_VERSION already present for $TARGET"
  exit 0
fi

mkdir -p "$DEST_DIR"
INTERMED_TMP="$(mktemp -d)"
trap 'rm -rf "$INTERMED_TMP"' EXIT

if [ -n "$ASSET" ]; then
  ARCHIVE="$INTERMED_TMP/$ASSET"
  URL="https://github.com/jarettr/intermed/releases/download/${INTERMED_VERSION}/${ASSET}"
  echo "==> Fetching InterMed $INTERMED_VERSION for $TARGET"
  curl -fSL "$URL" -o "$ARCHIVE"
  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_SHA="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
  else
    ACTUAL_SHA="$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')"
  fi
  if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
    echo "error: InterMed archive checksum mismatch" >&2
    exit 1
  fi
  mkdir -p "$INTERMED_TMP/unpack"
  case "$ASSET" in
    *.zip) unzip -q "$ARCHIVE" -d "$INTERMED_TMP/unpack" ;;
    *.tar.gz) tar -xzf "$ARCHIVE" -C "$INTERMED_TMP/unpack" ;;
  esac
  SOURCE_BIN="$(find "$INTERMED_TMP/unpack" -type f -name "intermed${EXE}" -print -quit)"
else
  echo "==> Building pinned InterMed $INTERMED_VERSION for $TARGET"
  git clone --filter=blob:none --no-checkout https://github.com/jarettr/intermed.git "$INTERMED_TMP/source"
  git -C "$INTERMED_TMP/source" checkout --detach "$INTERMED_COMMIT"
  ACTUAL_COMMIT="$(git -C "$INTERMED_TMP/source" rev-parse HEAD)"
  if [ "$ACTUAL_COMMIT" != "$INTERMED_COMMIT" ]; then
    echo "error: InterMed source revision mismatch" >&2
    exit 1
  fi
  cargo build --manifest-path "$INTERMED_TMP/source/Cargo.toml" \
    --locked --release --package intermed-cli --target "$TARGET"
  SOURCE_BIN="$INTERMED_TMP/source/target/$TARGET/release/intermed${EXE}"
fi

if [ ! -f "$SOURCE_BIN" ]; then
  echo "error: InterMed executable was not produced" >&2
  exit 1
fi
cp "$SOURCE_BIN" "$DEST_BIN"
chmod +x "$DEST_BIN"
if [ "$(uname -s)" = "Darwin" ]; then
  xattr -dr com.apple.quarantine "$DEST_BIN" 2>/dev/null || true
fi

VERSION_OUTPUT="$("$DEST_BIN" --version)"
case "$VERSION_OUTPUT" in
  "intermed 0.1.7-alpha") ;;
  *) echo "error: unexpected InterMed version '$VERSION_OUTPUT'" >&2; exit 1 ;;
esac
printf '%s' "$INTERMED_VERSION@$INTERMED_COMMIT:$TARGET" > "$STAMP"
echo "==> Bundled $VERSION_OUTPUT for $TARGET"
