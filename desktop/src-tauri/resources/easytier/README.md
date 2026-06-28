# Bundled EasyTier binaries

This directory ships the EasyTier binaries (`easytier-core` / `easytier-cli`) inside the
app so 联机大厅 works without users installing EasyTier separately.

The binaries are **not** committed (they are ~30MB and platform-specific). They are
downloaded for the build host by `scripts/fetch-easytier.sh`, which the release CI runs
before the Tauri build. The Tauri config bundles `resources/easytier/*` into the app's
resource dir under `easytier/`, where the launcher's binary resolver finds them
(macOS: `…/Contents/Resources/easytier/easytier-core`).

This README is tracked only so the `resources` glob always matches at least one file
(an empty/missing dir makes the Tauri build fail). `.gitignore` excludes everything else
in this directory.

To build a bundle locally:

```bash
scripts/fetch-easytier.sh        # downloads easytier-core + easytier-cli for your host
# then the usual tauri build
```

For dev (`scripts/dev-app.sh`), EasyTier is resolved from a sibling `target/debug/easytier/`
dir instead, so this directory can stay empty.
