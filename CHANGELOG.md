# Changelog

All notable changes to kobeMC are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

To cut a release, move the `[Unreleased]` notes into a stamped version section with
`scripts/release.sh <version>` (it also syncs the version across every manifest and tags it).

## [Unreleased]

## [0.1.7] - 2026-06-30

### Added
- Experimental modpack agent (`mc agent`): the customization step now analyzes the chosen base
  pack's actual mod list and credits the features it already covers, so it only plans the genuine
  gaps instead of piling redundant mods onto a pack that already fits.
- A local agent eval harness (`scripts/agent-eval.py`): runs intent/requirement-extraction test
  cases through a candidate model and scores them with a judge model.

### Changed
- The agent LLM defaults to `deepseek/deepseek-v4-pro` (override via `MC_AGENT_OPENROUTER_MODEL` /
  `OPENROUTER_MODEL`).
- Agent base-pack candidates are ranked by match relevance + popularity rather than smallest
  archive size, so the obvious popular match is no longer buried.
- Agent provider calls share a single HTTP client and run with bounded concurrency, plus a
  run-scoped version-lookup cache.

### Fixed
- The agent no longer adds a redundant mod for a feature the chosen base pack already provides.
- Transient `.mrpack` download/checksum failures during agent execution are now retried instead of
  aborting the whole build.
- The agent's requirement-normalization schema retry now fires on a real model parse error.

## [0.1.6] - 2026-06-28

### Fixed
- Concurrent installs into the shared library/asset store could corrupt or delete each other's
  in-progress downloads (the `.part` temp name was keyed only on the destination). Each writer now
  gets a unique temp file.
- CLI: CurseForge modpack imports failed with "需配置 API Key" even when a key was configured — the
  CLI now applies the CurseForge API key to its downloader and provider registry, matching the desktop app.
- Realm/server stubs declaring the `liteloader` or `optifine` loader were mis-detected as vanilla.
- Sharing an instance now surfaces the real server error instead of "share response missing id" when
  the publish request fails.

### Changed
- Internal: a 20-step architecture-deepening pass consolidated duplicated logic behind single owners
  (download temp naming, settings→downloader/registry construction, the `ServerClient` HTTP verbs,
  loader-family parsing, slug/collision naming, UUID formatting, account add→select→save, the
  version-stub writer, progress-channel wiring, and more). Behavior-preserving aside from the fixes above.

## [0.1.5] - 2026-06-28

### Fixed
- Fabric modpack/loader instances failed to launch ("Minecraft game provider couldn't locate the game!"): the Minecraft jar is now placed on the classpath by the vanilla base id instead of the leaf stub id.
- UI freeze ("卡一下") when opening an instance/modpack: heavy instance read commands (`list_instances`, mods/worlds/screenshots, memory suggestion) now run off the UI thread.

## [0.1.4] - 2026-06-28

## [0.1.3] - 2026-06-27

## [0.1.2] - 2026-06-27

## [0.1.1] - 2026-06-23

### Added

- **Saved servers** — each instance gains a "服务器 / Servers" tab listing the multiplayer servers
  from its `servers.dat` (name, address, icon). "Join" launches straight into a server via a
  one-shot `--quickPlayMultiplayer` override, without rewriting the instance config.

- **In-app log viewer** — Settings → 诊断 shows the tail of the unified log (bounded read), with refresh.
- **Skin preview** — full-body player skin render in the workspace account panel.
- **News feed** — the 动态 / classic news sections now show the launcher feed from mc-server (degrades to empty when the server is unavailable).
- **Modpack update** — instances installed from a Modrinth modpack show an "update available"
  chip; clicking it updates the pack in place (re-imports the new version over the existing
  instance and trashes mods the new version dropped). Worlds, instance config, and
  user-added mods are preserved.

### Security / hardening

- mrpack download URLs are now restricted to the documented host allowlist
  (cdn.modrinth.com / github.com / raw.githubusercontent.com / gitlab.com), so a crafted pack
  can't fetch from arbitrary hosts.
- Batch downloads now honour the configured concurrency instead of the momentarily-available
  permit count (no throttling when a Downloader is shared).
- Server addresses with IPv6 literals (`[::1]:25565`) parse correctly for auto-join.

## [0.1.0] - 2026-06-23

First public build of kobeMC — a from-scratch, cross-platform Minecraft launcher
(Rust core + Tauri v2 shell + SolidJS UI).

### Added

- **Accounts** — offline, Microsoft (device-code flow), and Yggdrasil / authlib-injector
  (third-party skin sites); switch and remove accounts from one shared dialog.
- **Instances** — create with Vanilla / Fabric / Quilt / Forge / NeoForge (loader versions
  fetched into a picker, no hand-typed build numbers); copy, delete (to the recycle bin),
  import & export `.mrpack`, and manage settings / Mods / resource packs / shaders / data packs /
  worlds / screenshots per instance.
- **Discover** — browse, search, and install from Modrinth (modpacks, mods, shaders, resource
  packs, data packs); CurseForge modpack import with a guided manual-download list for
  author-restricted files.
- **Launch** — auto-provisions vanilla + the chosen loader + a matching Temurin JRE; live
  progress, real running/stop state, and crash diagnostics.
- **Game directories** — discover multiple roots, switch between them, add/remove custom roots;
  the selected root persists across restarts.
- **Two UIs** — a dark "workspace" layout and a PCL-faithful light "classic" layout, switchable
  at runtime; HSL live theming and an adjustable window veil.
- **Bilingual UI** — full 中文 / English via @solid-primitives/i18n (default Chinese), switchable
  in Settings and persisted.
- **Quality of life** — instance search & recent-first ordering, a unified daily-rolling log,
  and confirmations before destructive actions.

[Unreleased]: https://github.com/Sma1lboy/mc/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Sma1lboy/mc/releases/tag/v0.1.0
