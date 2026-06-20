# Architecture deepening pass (2026-06): what we deepened, and what we deliberately did not

A `/improve-codebase-architecture` review surfaced ~9 deepening candidates. We landed every one whose
design was determined and could be made behavior-preserving + test/build-guarded, and recorded a
decision on the handful that genuinely cannot be done without missing infrastructure or a visual
verification loop.

## Landed (committed this pass)

- **One `inheritsFrom` chain traversal** — `version::walk_inherits` replaces two divergent walks
  (instance-listing lenient depth-16 vs launch strict depth-32) with one guarded traversal that also
  gained cycle detection. `load_chain` / `resolve_base_mc_version` are thin adapters. (+ tests)
- **Concentrated loader-install choreography** — `installer::ensure_vanilla` / `finalize` /
  `install_via_jar`; the four `install_*` functions now hold only their real per-loader variance.
- **`Menu` UI adapter** — inline Ark Menu in InstanceRow + ContextBar is one house-styled seam;
  all `@ark-ui/solid/menu` access lives in `components/Menu.tsx`.
- **Per-layout route table** (`routes.ts`) — both shells render the current page via `<Dynamic>` from a
  declarative table instead of hand-written `Switch/Match`.
- **`activeRoot()`** — the "no root → pass `\"\"`" convention lives once instead of at 16 call sites.
- **Shared archive-path helpers** (`basename`/`depth`/`shallowest_marker`) hoisted out of the four
  import adapters.
- Boot theme routed through the single `themeForLayout` seam.

## Decided NOT to do without more input (load-bearing)

- **`RuntimeContext.os_version` is still intentionally left empty — do not populate it casually.**
  The *matcher* is now fixed and tested (`os_version_prefix` unescapes `\.`; previously a populated
  version could never match) — but that change is inert at runtime because `os_version` stays empty.
  Actually populating it (a) needs a real cross-platform OS-version detector, which does not exist in
  the tree, and (b) makes `os.version` library/argument rules start firing, changing which libraries
  and args land on the classpath — a launch-filtering change that can only be trusted after verifying a
  real game launch on each OS. Do that piece with a human who can launch the game.
  `RuntimeContext.features` likewise stays `default` until the launcher exposes demo-mode /
  custom-resolution options to feed it.
- **Frontend load/error *surfacing* is not yet standardized.** The `activeRoot()` threading is done, but
  unifying how each page shows loading/empty/error (today some `.catch(()=>[])` silently, some toast)
  would change per-page visual behavior that can only be verified by watching the app — do it with a
  human in the loop, not blind.
- **`ResourceProvider` seam: the MCBBS↔CurseForge coupling is left as-is.** The MCBBS importer calls
  CurseForge resolution by name because the MCBBS pack format *is* CurseForge refs + overrides — the
  coupling is domain, not accidental. Fully hiding it behind the trait is a real interface redesign with
  semantic implications; revisit deliberately, not as a mechanical refactor.
