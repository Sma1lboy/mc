# Changelog

All notable changes to kobeMC are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

To cut a release, move the `[Unreleased]` notes into a stamped version section with
`scripts/release.sh <version>` (it also syncs the version across every manifest and tags it).

## [Unreleased]

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
- **Quality of life** — Chinese UI throughout, instance search & recent-first ordering, a
  unified daily-rolling log, and confirmations before destructive actions.

[Unreleased]: https://github.com/Sma1lboy/mc/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Sma1lboy/mc/releases/tag/v0.1.0
