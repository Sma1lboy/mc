#!/usr/bin/env bash
#
# release.sh — cut a kobeMC release.
#
# Bumps the version across every manifest (the version lives in 4 places),
# syncs the lockfiles, stamps CHANGELOG.md, runs the same checks as CI, then
# commits and creates an annotated `v<version>` tag.
#
#   scripts/release.sh 0.1.0            # prepare + commit + tag locally
#   scripts/release.sh 0.2.0 --skip-checks   # skip cargo test / frontend build
#
# Nothing is pushed. Review, then:
#   git push --follow-tags             # → triggers .github/workflows/release.yml
#
# Note: this script only prepares the version bump + tag. The platform bundles are
# built in CI (.github/workflows/release.yml), which runs scripts/fetch-easytier.sh
# to download + bundle the EasyTier binaries (联机大厅) before the tauri build. To
# build a bundle locally, run `scripts/fetch-easytier.sh` first, then the tauri build.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-}"
SKIP_CHECKS=0
[ "${2:-}" = "--skip-checks" ] && SKIP_CHECKS=1

if [ -z "$VERSION" ]; then
  echo "usage: scripts/release.sh <version> [--skip-checks]   e.g. scripts/release.sh 0.1.0" >&2
  exit 2
fi
# SemVer (optionally with -pre / +build metadata).
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
  echo "error: '$VERSION' is not a valid semver (expected MAJOR.MINOR.PATCH)" >&2
  exit 2
fi

TAG="v$VERSION"
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "error: tag $TAG already exists" >&2
  exit 1
fi

echo "==> Setting version to $VERSION across manifests"
VERSION="$VERSION" python3 - <<'PY'
import json, os, re, sys

version = os.environ["VERSION"]

def replace_toml_version(path, section):
    """Replace the first `version = "..."` after [section] in a Cargo.toml."""
    with open(path, encoding="utf-8") as f:
        text = f.read()
    pat = re.compile(r'(\[' + re.escape(section) + r'\][^\[]*?\nversion\s*=\s*")[^"]*(")', re.S)
    new, n = pat.subn(lambda m: m.group(1) + version + m.group(2), text, count=1)
    if n != 1:
        sys.exit(f"error: could not find version under [{section}] in {path}")
    with open(path, "w", encoding="utf-8") as f:
        f.write(new)
    print(f"   {path} [{section}] -> {version}")

def replace_json_version(path, keys):
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    for k in keys:
        # k is a path like ["version"] or ["packages","","version"]
        node = data
        for part in k[:-1]:
            if part not in node:
                break
            node = node[part]
        else:
            if k[-1] in node:
                node[k[-1]] = version
    with open(path, "w", encoding="utf-8") as f:
        json.dump(data, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print(f"   {path} -> {version}")

replace_toml_version("Cargo.toml", "workspace.package")
replace_toml_version("desktop/src-tauri/Cargo.toml", "package")
replace_json_version("desktop/src-tauri/tauri.conf.json", [["version"]])
replace_json_version("desktop/package.json", [["version"]])
# npm workspaces:唯一真相是仓库根 package-lock.json(desktop/package-lock.json 已废弃)。
replace_json_version("package-lock.json", [["packages", "desktop", "version"]])
PY

echo "==> Syncing lockfiles"
# `cargo metadata` rewrites the member version in Cargo.lock without touching deps.
cargo metadata --format-version 1 -q >/dev/null
cargo metadata --format-version 1 -q --manifest-path desktop/src-tauri/Cargo.toml >/dev/null

echo "==> Stamping CHANGELOG.md"
VERSION="$VERSION" python3 - <<'PY'
import datetime, os, re
version = os.environ["VERSION"]
path = "CHANGELOG.md"
with open(path, encoding="utf-8") as f:
    text = f.read()
if re.search(r'^## \[' + re.escape(version) + r'\]', text, re.M):
    print(f"   [{version}] section already present, leaving as-is")
else:
    today = datetime.date.today().isoformat()
    # Promote the Unreleased notes to a stamped section, leave a fresh Unreleased.
    text = text.replace(
        "## [Unreleased]",
        f"## [Unreleased]\n\n## [{version}] - {today}",
        1,
    )
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"   added ## [{version}] - {today}")
PY

if [ "$SKIP_CHECKS" -eq 0 ]; then
  echo "==> Running checks (cargo test + frontend build)"
  cargo test --workspace --quiet
  ( cd desktop && npm ci --silent && npm run build )
else
  echo "==> Skipping checks (--skip-checks)"
fi

echo "==> Committing + tagging $TAG"
# Note: Cargo.lock files are gitignored in this repo, so they are not staged here.
git add Cargo.toml \
        desktop/src-tauri/Cargo.toml \
        desktop/src-tauri/tauri.conf.json \
        desktop/package.json desktop/package-lock.json \
        CHANGELOG.md
# Nothing changed when the version is already current (e.g. the very first release) —
# skip the empty commit and just tag the current HEAD.
if git diff --cached --quiet; then
  echo "   version already current — nothing to commit; tagging HEAD"
else
  git commit -m "chore(release): $TAG"
fi
git tag -a "$TAG" -m "kobeMC $TAG"

cat <<EOF

✓ Prepared $TAG (committed + tagged locally, nothing pushed).

Next:
  git push --follow-tags        # pushes the commit + tag → builds bundles via GitHub Actions
                                # (the release is published directly — never a draft)
EOF
