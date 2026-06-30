# Architecture deepening pass #2 (2026-06, agent + cross-cutting): what we deepened, what is queued

A second `/improve-codebase-architecture` review ran after the agent subsystem landed (the whole
`crates/mc-core/src/agent/` tree post-dates ADR-0001, so it had never been reviewed). Six areas were
explored; we landed five behavior-preserving deepenings this round, each in a disjoint file set so they
could be implemented in parallel and verified independently (clippy `-D warnings` + the full `mc-core`
suite, 0 regressions; the desktop crate + `export_bindings` also re-verified — no bindings drift).

## Landed (committed this pass)

- **`AgentRunSnapshot` phase-transition methods** — `request_approval` / `enter_phase` / `complete` /
  `fail` on the snapshot now own the `status ⇔ pending_approval ⇔ phase` invariant the
  agent-tauri-contract depends on. ~16 ad-hoc field-cluster sites across base_search / customization /
  requirements / execution / workflow route through them. `run.tools` handling and
  `approved_build`/`execution`/`mod_plan` clearing were left untouched (deliberately inconsistent /
  owned elsewhere) — pure consolidation, not unification.
- **`fs::trash_or_delete`** — the "try recycle-bin, else hard-delete (dir vs file)" nugget, hand-rolled
  in 6 resource-delete sites, now has one owner. Per-site `exists()` idempotence guards stayed put.
- **`crate::host`** — one `host_of` (scheme/userinfo/port strip) + `host_matches_suffix` (anti-spoof)
  owner replaces 3 hand-rolled parsers + 3 anti-spoof copies; the mrpack download-host allowlist is now
  a single const in `formats::mrpack` (was duplicated in import + export). Also fixed a latent bug:
  `Downloader::get_bytes_capped` now attaches CurseForge `x-api-key` like the other two fetch methods.
- **`StoredAccount::from_microsoft_refreshed`** — the Microsoft refresh path stopped re-listing ~11
  fields + re-deriving the TTL inline; both the initial and refresh paths now share one owner, killing
  the drift the constructor's own doc-comment warns about.
- **Shared `java_exe_name`** — the verbatim-duplicated exe-name helper is defined once. (The `bin/` vs
  `Contents/Home/bin/` *layout* probes were deliberately NOT unified — detect and install probe
  different sets, so sharing them would change detection behavior; do that with intent, not mechanically.)

## Queued for a next round (deferred only because they overlap the agent files above)

These passed the deletion test but touch the same agent/workflow files as the transition-methods
deepening, so they could not run in the same parallel round. They are intentionally next, not missed:

- **`BuildRestrictions::apply`** — promote patch-application (revision check, history, normalization,
  missing/warnings) onto the type. Highest-value of the remainder: it collapses a **real latent drift**
  — restriction normalization runs twice today (`normalize_restriction_update_input` silently drops an
  invalid mc_version + trims tags; the inline pass in `update_build_restrictions` re-validates, warns,
  and lowercases) — into one path.
- **`ProviderRegistry::search_concurrent`** — fold the twice-copied bounded-concurrency + (provider,id)
  dedup + cap logic (base_search ↔ customization) behind one registry method; also unifies the three
  divergent `(provider, id)` identity-key spellings.
- **Inject one `ProviderRegistry`** into the agent workflow instead of `with_defaults()` rebuilt at 6
  sites (kills the documented "silent CF-unavailable" footgun; lets the FakeProvider registry drive the
  real search/version paths in tests).
- **Execution-manifest vocabulary + typed constructors** — end the stringly-typed status / replan_phase
  / error_kind dispatch split across producers and consumers (keep `serde_json::Value` at the boundary;
  this is vocabulary concentration, not a wire-format change).

## Build-hygiene note (not a code change)

A fresh `Cargo.lock` resolution currently pulls `time 0.3.52`, whose breaking `Parsable::parse` signature
fails to compile transitive `cookie 0.18.1` (via reqwest's `cookies` feature). `Cargo.lock` is gitignored,
so this bites fresh checkouts / new worktrees / CI's own resolve, not committed code. Pin `time` (e.g. a
`=0.3.51` constraint in a tracked manifest) if CI starts failing at the `cookie` compile.
