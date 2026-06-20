# Architecture deepening pass (2026-06): what we deepened, and what we deliberately did not

A `/improve-codebase-architecture` review surfaced ~9 deepening candidates. We landed the ones whose
design was determined and verifiable without launch-critical risk, and recorded a decision on the rest
rather than auto-refactoring them. The remaining candidates are not "rejected" — they want the skill's
collaborative grilling loop, not a unilateral change.

## Landed (committed this pass)

- **One `inheritsFrom` chain traversal** — `version::walk_inherits` replaces two divergent walks
  (instance-listing: lenient, depth-16, lite head parse; launch: strict, depth-32, full json) with one
  guarded traversal that *also* gained cycle detection. `load_chain` and `resolve_base_mc_version` are
  now thin adapters supplying only node-read + error policy.
- **`Menu` UI adapter** — the inline Ark Menu in `InstanceRow` + `ContextBar` is one house-styled seam
  alongside Select/Tooltip/Dialog; all `@ark-ui/solid/menu` access lives in `components/Menu.tsx`.
- **Shared archive-path helpers** — `basename`/`depth`/`shallowest_marker` hoisted out of the four
  import adapters into `modpack::import`.
- Boot theme routed through the single `themeForLayout` seam (last hardcoded `PCL_THEME` removed).

## Deferred to grilling (worth doing, but not solo)

- **Loader installation seam.** The four `install_*` functions share a choreography (ensure vanilla →
  resolve loader version → write profile → resolve + ensure files) worth concentrating. But these paths
  run installer jars / hit the network and have **no unit-test coverage** — a trait-or-helpers refactor
  here is launch-critical and unguarded. Do it with a human and an integration test, not blind.
- **Frontend data-loading seam.** Concentrating `currentRoot` threading + load/error is worthwhile, but
  standardizing error surfacing changes per-page toast behavior that can only be verified visually.
- **`ResourceProvider` seam tightening.** Pushing `resolve_curseforge_refs` / the blocked-file concept
  behind the trait (so deleting CurseForge can't break the MCBBS importer) is a genuine interface
  redesign.

## Decided NOT to do (load-bearing)

- **`RuntimeContext.os_version` is intentionally left empty — do not "fix" it by populating it.**
  It looks like a one-line bug. It is not. There is no OS-version detector anywhere in the tree, and
  making `os.version` library/argument rules actually fire changes which libraries and args land on the
  classpath — an untestable change to launch filtering. A correct fix needs a real cross-platform
  OS-version source **plus** tests first. Same for `RuntimeContext.features`: it stays `default` until
  the launcher actually exposes demo-mode / custom-resolution options to feed it.
- **Routing stays as per-shell `Switch/Match(currentPage())`; we declined a route registry.**
  A route table would add an indirection layer for marginal locality gain at ~4 pages per layout, and
  the two shells differ in chrome anyway (Modrinth: Rail + ContextBar; PCL: top tabs). Revisit only if
  the page count or layout count grows materially.
