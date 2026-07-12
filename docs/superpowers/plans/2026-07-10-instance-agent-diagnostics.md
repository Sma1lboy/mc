# Instance Agent Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add entry-specific build and instance agent tools for structured instance diagnosis, mandatory build compatibility validation, and user-confirmed instance changes.

**Architecture:** Rust owns compatibility facts and all mutations. The TypeScript agent package owns prompts and schemas, while the desktop injects instance context and renders the only mutation approval surface. Build and instance remain separate entry profiles with complete relevant tool sets and ordinary model-selected tool calling.

**Tech Stack:** Rust 2021, Tokio, serde, Tauri v2, specta, TypeScript, AI SDK, React 19, Vitest-style TypeScript tests, Cargo unit tests.

## Global Constraints

- Keep explicit `build` and instance-bound `instance` entry profiles.
- Do not add progressive tool disclosure or `activate_tools`.
- Model schemas must not accept game roots, instance IDs, or local paths owned by the host.
- Real-instance writes require a visible user confirmation card.
- `build_modpack` must enforce blocking compatibility results internally.
- Do not expose shell or arbitrary filesystem tools.
- Sandbox test-launch automation is out of scope.

---

### Task 1: Shared Compatibility Types And Instance Diagnosis

**Files:**
- Create: `crates/mc-core/src/agent/compatibility.rs`
- Create: `crates/mc-core/src/agent/tools/diagnose_instance.rs`
- Modify: `crates/mc-core/src/agent/mod.rs`
- Modify: `crates/mc-core/src/agent/tools/mod.rs`
- Modify: `crates/mc-core/src/agent/tools/tests.rs`

**Interfaces:**
- Produces: `CompatibilityReport`, `CompatibilityIssue`, `CompatibilityStatus`, `CompatibilitySeverity`, `SuggestedAction`.
- Produces: `DiagnoseInstanceArgs`, `DiagnoseInstanceOutput`, `tool_diagnose_instance(paths, instance_id, args)`.
- Consumes: `GamePaths`, `Instance`, `list_instances`, `list_mods`, `suggest_memory_mb`, and `diagnostics::analyze`.

- [ ] **Step 1: Add failing compatibility status tests**

Add tests proving an empty report is healthy, warnings produce warning status,
and any blocking issue produces blocked status:

```rust
#[test]
fn compatibility_report_status_follows_highest_severity() {
    assert_eq!(CompatibilityReport::from_issues(vec![]).status, CompatibilityStatus::Healthy);
    assert_eq!(
        CompatibilityReport::from_issues(vec![CompatibilityIssue::warning("x", "warning")]).status,
        CompatibilityStatus::Warning,
    );
    assert_eq!(
        CompatibilityReport::from_issues(vec![CompatibilityIssue::blocking("x", "blocked")]).status,
        CompatibilityStatus::Blocked,
    );
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test -p mc-core compatibility_report_status_follows_highest_severity`

Expected: compilation fails because the compatibility types do not exist.

- [ ] **Step 3: Implement the shared report contract**

Create serde/specta types using `snake_case` wire values and deterministic
status derivation:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus { Healthy, Warning, Blocked }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilitySeverity { Info, Warning, Blocking }

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct SuggestedAction {
    pub kind: String,
    pub target: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CompatibilityIssue {
    pub code: String,
    pub severity: CompatibilitySeverity,
    pub summary: String,
    pub subjects: Vec<String>,
    pub evidence: Vec<String>,
    pub suggested_actions: Vec<SuggestedAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CompatibilityReport {
    pub status: CompatibilityStatus,
    pub issues: Vec<CompatibilityIssue>,
}
```

- [ ] **Step 4: Add failing instance-diagnosis tests**

Use a temporary `GamePaths` fixture with `instance.json`, local Mod jars, and
`logs/latest.log`. Cover duplicate Mod IDs, Fabric-on-Forge mismatch, low
memory, crash analysis, and absent logs.

```rust
#[tokio::test]
async fn diagnose_instance_reports_duplicate_ids_loader_memory_and_crash() {
    let fixture = diagnostic_instance_fixture("forge");
    fixture.write_mod("one.jar", r#"{"id":"same","version":"1"}"#);
    fixture.write_mod("two.jar", r#"{"id":"same","version":"2"}"#);
    fixture.write_latest_log("java.lang.OutOfMemoryError: Java heap space");
    let out = tool_diagnose_instance(
        &fixture.paths,
        &fixture.id,
        DiagnoseInstanceArgs { include_log_tail: true },
    ).await.unwrap();
    let codes = out.report.issues.iter().map(|issue| issue.code.as_str()).collect::<Vec<_>>();
    assert!(codes.contains(&"duplicate_mod_id"));
    assert!(codes.contains(&"mod_loader_mismatch"));
    assert!(codes.contains(&"last_launch_crash"));
}
```

- [ ] **Step 5: Run instance-diagnosis tests and verify RED**

Run: `cargo test -p mc-core diagnose_instance_`

Expected: compilation fails because `tool_diagnose_instance` does not exist.

- [ ] **Step 6: Implement bounded log collection and diagnosis**

Implement a maximum 512 KiB read and a maximum 200 returned lines. Prefer
`logs/latest.log`, then the newest regular file under `crash-reports/`. Build
issues only from evidence actually present. Use `LoaderKind::as_str()` and the
existing Quilt-accepts-Fabric rule.

Keep the deterministic checks in an internal
`diagnose_instance_with_total_memory(paths, id, args, total_mb)` helper so
tests supply a fixed memory total; the public tool passes
`system_total_mem_mb()`.

- [ ] **Step 7: Run focused and module tests**

Run: `cargo test -p mc-core agent::tools::tests::diagnose_instance_`

Expected: all new diagnosis tests pass.

- [ ] **Step 8: Commit Task 1**

```bash
git add crates/mc-core/src/agent/compatibility.rs \
  crates/mc-core/src/agent/mod.rs \
  crates/mc-core/src/agent/tools/diagnose_instance.rs \
  crates/mc-core/src/agent/tools/mod.rs \
  crates/mc-core/src/agent/tools/tests.rs
git commit -m "feat(agent): add structured instance diagnostics"
```

### Task 2: Build Plan Validation And Mandatory Build Gate

**Files:**
- Create: `crates/mc-core/src/agent/tools/validate_modpack_plan.rs`
- Modify: `crates/mc-core/src/agent/tools/build_modpack.rs`
- Modify: `crates/mc-core/src/agent/tools/mod.rs`
- Modify: `crates/mc-core/src/agent/tools/tests.rs`

**Interfaces:**
- Consumes: compatibility report types from Task 1.
- Produces: `ValidateModpackPlanArgs`, `ValidateModpackPlanOutput`, `validate_modpack_plan`, and `tool_validate_modpack_plan`.
- Changes: `tool_build_modpack` calls the validator before resolving/writing output.

- [ ] **Step 1: Add failing validation tests**

Use `FakeChatProvider` exact versions to cover duplicate projects, target
incompatibility, missing required dependencies, and an incompatible edge whose
target is actually selected.

```rust
#[tokio::test]
async fn validate_modpack_plan_blocks_present_declared_conflict() {
    let ctx = conflict_provider_context();
    let out = tool_validate_modpack_plan(&ctx, conflict_plan_args()).await.unwrap();
    assert_eq!(out.report.status, CompatibilityStatus::Blocked);
    assert!(out.report.issues.iter().any(|issue| issue.code == "declared_mod_conflict"));
}
```

Also add a build test that asserts no output file exists after a blocked plan.

- [ ] **Step 2: Run focused validation tests and verify RED**

Run: `cargo test -p mc-core validate_modpack_plan_`

Expected: compilation fails because validation APIs do not exist.

- [ ] **Step 3: Implement exact-version graph validation**

Normalize keys as `<provider>:<project_id>`. Fetch each project's versions,
select only the requested `version_id`, verify target MC/loader support and a
primary file, then evaluate `required` and `incompatible` edges against the
selected/base key set. Return provider failures as `ChatToolError`.

```rust
pub async fn validate_modpack_plan(
    ctx: &ChatToolsCtx,
    args: &ValidateModpackPlanArgs,
) -> Result<CompatibilityReport, ChatToolError>;
```

- [ ] **Step 4: Reuse base archive project extraction**

Factor the exact base archive fetch from `inspect_base_modpack.rs` into a
`pub(super)` helper returning `(ProjectVersion, Vec<u8>)`. Validation must use
the exact `BuildBasePack.version_id`, not whichever version happens to be first.

- [ ] **Step 5: Add the mandatory build gate**

At the beginning of `tool_build_modpack`, convert its args to validation args.
If blocked, return:

```rust
BuildModpackOutput {
    status: "blocked".into(),
    output_path: None,
    output_size: None,
    manifest: serde_json::json!({
        "schema_version": 1,
        "status": "blocked",
        "reason": "modpack compatibility validation failed",
        "compatibility": report,
    }),
}
```

- [ ] **Step 6: Run build and tool tests**

Run: `cargo test -p mc-core agent::tools`

Expected: all agent tool tests pass and blocked plans do not write files.

- [ ] **Step 7: Commit Task 2**

```bash
git add crates/mc-core/src/agent/tools/validate_modpack_plan.rs \
  crates/mc-core/src/agent/tools/inspect_base_modpack.rs \
  crates/mc-core/src/agent/tools/build_modpack.rs \
  crates/mc-core/src/agent/tools/mod.rs \
  crates/mc-core/src/agent/tools/tests.rs
git commit -m "feat(agent): validate modpack compatibility before build"
```

### Task 3: Agent Entry Prompts And Tool Schemas

**Files:**
- Create: `packages/agent-core/src/tools/diagnose-instance.ts`
- Create: `packages/agent-core/src/tools/validate-modpack-plan.ts`
- Create: `packages/agent-core/src/tools/show-instance-changes.ts`
- Modify: `packages/agent-core/src/types.ts`
- Modify: `packages/agent-core/src/tools/index.ts`
- Modify: `packages/agent-core/src/prompt.ts`
- Modify: `packages/agent-core/test/agent.test.ts`

**Interfaces:**
- Produces: canonical entries `build` and `instance`, while parsing legacy `modpack` and `wiki` values.
- Produces: schemas for the three new tools; instance schemas contain no host paths or IDs.

- [ ] **Step 1: Add failing entry/tool tests**

```ts
it("exposes build validation only in build entry", () => {
  expect(Object.keys(buildTools("build"))).toContain("validate_modpack_plan");
  expect(Object.keys(buildTools("build"))).not.toContain("diagnose_instance");
});

it("exposes instance diagnosis, wiki, provider, and change tools", () => {
  const names = Object.keys(buildTools("instance"));
  expect(names).toEqual(expect.arrayContaining([
    "wiki_search", "wiki_open", "diagnose_instance", "search_mods",
    "mod_get_detail", "resolve_mods", "show_instance_changes",
  ]));
});
```

- [ ] **Step 2: Run tests and verify RED**

Run: `npm test --workspace @kobemc/agent-core`

Expected: tests fail because new entries and tools are absent.

- [ ] **Step 3: Implement strict schemas**

`diagnose_instance` accepts only `include_log_tail`. `validate_modpack_plan`
reuses target/base/extra shapes. `show_instance_changes` uses a discriminated
union for the four initial operations and requires a human-readable reason per
operation.

- [ ] **Step 4: Split prompt selection by entry**

Keep a shared grounding/safety suffix and explicit entry prompts:

```ts
export type AgentEntry = "build" | "instance";
export function promptForEntry(entry: AgentEntry): string;
export function buildTools(entry: AgentEntry): ToolSet;
```

Retain `AgentMode` as a deprecated input alias during persisted-data migration.

- [ ] **Step 5: Run agent-core tests**

Run: `npm test --workspace @kobemc/agent-core`

Expected: all tests pass.

- [ ] **Step 6: Commit Task 3**

```bash
git add packages/agent-core/src packages/agent-core/test/agent.test.ts
git commit -m "feat(agent): add build and instance tool profiles"
```

### Task 4: Tauri Commands And Desktop Dispatch

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Regenerate: `desktop/src/ipc/bindings.ts`
- Modify: `desktop/src/agent/chatStore.ts`
- Modify: `desktop/src/agent/clientToolDispatcher.ts`
- Modify: `desktop/src/agent/desktopAdapter.ts`
- Modify: `desktop/src/agent/localRuntimeAdapter.ts`
- Modify: `packages/agent-core/bin/harness-host.mjs`

**Interfaces:**
- Adds Tauri commands `agent_tool_diagnose_instance` and `agent_tool_validate_modpack_plan`.
- Extends `AgentToolContext` with a required host-bound instance payload for `instance` entry.
- Registers the new automatic and interactive tool names in both runtimes.

- [ ] **Step 1: Add compile-time command wiring**

Add thin Tauri wrappers. Diagnosis receives `root` and `instance_id` as
separate host arguments; the model-owned args remain path-free.

- [ ] **Step 2: Update dispatcher guards**

Map build and instance entries to exact tool-name sets. Inject root/ID for
diagnosis and forward validation directly to the shared `AgentToolsState`.

- [ ] **Step 3: Migrate entry naming at adapters**

Map legacy `modpack -> build` and `wiki -> instance` at load boundaries. New
conversations persist only canonical entry names.

- [ ] **Step 4: Regenerate IPC bindings and compile**

Run: `cargo check --manifest-path desktop/src-tauri/Cargo.toml`

Expected: successful check and updated generated bindings containing both new
commands and compatibility DTOs.

- [ ] **Step 5: Build the desktop frontend**

Run: `npm run build --workspace desktop`

Expected: Vite production build succeeds.

- [ ] **Step 6: Commit Task 4**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/lib.rs \
  desktop/src/ipc/bindings.ts desktop/src/agent/chatStore.ts \
  desktop/src/agent/clientToolDispatcher.ts desktop/src/agent/desktopAdapter.ts \
  desktop/src/agent/localRuntimeAdapter.ts packages/agent-core/bin/harness-host.mjs
git commit -m "feat(agent): wire instance diagnostics into desktop"
```

### Task 5: User-Confirmed Instance Change Card

**Files:**
- Create: `desktop/src/agent/InstanceChangesCard.tsx`
- Modify: `desktop/src/agent/ChatParts.tsx`
- Modify: `desktop/src/agent/MessageList.tsx`
- Modify: `desktop/src/agent/chatStore.ts`
- Modify: `desktop/src/agent/clientToolDispatcher.ts`
- Modify: `desktop/src/locales/agent.ts`

**Interfaces:**
- Consumes: `show_instance_changes` tool input from Task 3 and bound instance context from Task 4.
- Produces: tool result `{ applied: boolean, completed: string[], error?: string }`.

- [ ] **Step 1: Add pure operation validation tests**

Extract input normalization to a small exported function and test memory bounds,
safe file names, supported providers, and empty plans before rendering.

- [ ] **Step 2: Implement the card states**

Match existing `ModpackCard` semantics: skeleton while streaming, action list
while awaiting input, disabled buttons while busy, completed/skipped result,
and inline first-error display.

- [ ] **Step 3: Execute confirmed operations sequentially**

Reuse generated commands:

```ts
set_memory       -> getInstanceConfig + setInstanceConfig
set_mod_enabled  -> setModEnabled
delete_mod       -> deleteMod
install_mod      -> installMod using bound mcVersion and loader
```

Stop at the first failure and return only actually completed action labels.

- [ ] **Step 4: Integrate the interactive pause channel**

Add `show_instance_changes` to `INTERACTIVE_CLIENT_TOOLS`, local pending-tool
resolution, `isActivity` exclusions, and `MessageList` card rendering.

- [ ] **Step 5: Build and inspect UI stories**

Run: `npm run build --workspace desktop`

Expected: build succeeds. Add or update the existing agent stories to cover
pending, applied, skipped, and failed card states if the story harness already
supports tool parts.

- [ ] **Step 6: Commit Task 5**

```bash
git add desktop/src/agent/InstanceChangesCard.tsx desktop/src/agent/ChatParts.tsx \
  desktop/src/agent/MessageList.tsx desktop/src/agent/chatStore.ts \
  desktop/src/agent/clientToolDispatcher.ts desktop/src/locales/agent.ts
git commit -m "feat(agent): add confirmed instance change plans"
```

### Task 6: Regression Evals And Full Verification

**Files:**
- Modify: `packages/agent-core/test/agent.test.ts`
- Create: `scripts/instance-agent-eval.mjs`
- Modify: `packages/agent-core/package.json`
- Modify: `packages/agent-core/README.md`

**Interfaces:**
- Produces: fixture-driven `eval:instance` command with machine-readable JSON output.

- [ ] **Step 1: Add deterministic routing fixtures**

Cases: recipe -> Wiki, crash -> diagnosis, requested Mod change -> confirmation,
custom build -> validate before build. Assert tool names and absence of direct
mutation claims.

- [ ] **Step 2: Run focused evals**

Run: `npm run eval:instance --workspace @kobemc/agent-core -- --json`

Expected: all fixture cases pass.

- [ ] **Step 3: Run full verification**

```bash
cargo test -p mc-core agent::
npm test --workspace @kobemc/agent-core
npm run eval:instance --workspace @kobemc/agent-core -- --json
npm run build --workspace desktop
git diff --check
```

Expected: every command exits zero and `git diff --check` prints nothing.

- [ ] **Step 4: Review final diff for unrelated changes**

Run: `git status --short && git diff --stat && git diff`

Expected: only files listed by this plan are changed.

- [ ] **Step 5: Commit Task 6**

```bash
git add packages/agent-core/test/agent.test.ts packages/agent-core/package.json \
  packages/agent-core/README.md scripts/instance-agent-eval.mjs
git commit -m "test(agent): cover instance diagnosis workflows"
```
