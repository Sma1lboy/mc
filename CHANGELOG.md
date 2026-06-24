# Changelog

All notable changes to kobeMC are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

To cut a release, move the `[Unreleased]` notes into a stamped version section with
`scripts/release.sh <version>` (it also syncs the version across every manifest and tags it).

## [Unreleased]

## [0.2.0] - 2026-06-23

### Added

- **Saved servers** — each instance gains a "服务器 / Servers" tab listing the multiplayer servers
  from its `servers.dat` (name, address, icon). "Join" launches straight into a server via a
  one-shot `--quickPlayMultiplayer` override, without rewriting the instance config.

- **In-app log viewer** — Settings → 诊断 shows the tail of the unified log (bounded read), with refresh.
- **Skin preview** — full-body player skin render in the workspace account panel.
- **News feed** — the 动态 / classic news sections now show the launcher feed from mc-server (degrades to empty when the server is unavailable).
- **Modpack update** — instances installed from a Modrinth modpack show an "update available"
  chip; clicking it shows the new version's changelog and updates the pack in place (re-imports the
  new version over the existing instance and trashes mods the new version dropped). Worlds, instance
  config, and user-added mods are preserved.
- **Import dialog** — importing a modpack now shows the supported formats (Modrinth `.mrpack`,
  CurseForge `.zip`, MultiMC / Prism, MCBBS), drag-and-drop, and import tips. Dragging a
  `.mrpack`/`.zip` onto Home or the Library imports it directly, with progress.
- **Export dialog** — export an instance as a Modrinth `.mrpack`, a CurseForge `.zip`, or a mod
  list (Markdown / HTML / JSON / CSV / plain text), instead of Modrinth-only.

### Changed

- Smaller release build — the desktop app binary shrank from ~7 MB to ~5.6 MB via a size-tuned
  release profile (the app crate is a separate workspace and was building with Cargo defaults),
  trimmed `tokio` features, and `zip` reduced to the deflate codec.

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
