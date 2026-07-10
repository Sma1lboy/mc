# Instance Agent Diagnostics And Build Validation

Date: 2026-07-10

## Summary

kobeMC keeps two explicit agent entry profiles:

- `build`: create a new modpack from provider data.
- `instance`: assist one host-bound installed instance.

Each entry injects its own prompt and receives its complete relevant tool set.
The model chooses tools directly through normal automatic tool calling. There
is no deferred tool activation or progressive disclosure layer.

This change adds a shared structured compatibility report, an instance
diagnostic tool, mandatory build-plan validation, and a user-confirmed instance
change card. The same deterministic facts support both agent entries.

## Entry Profiles

### Build entry

The build entry has no bound instance. It retains the existing discovery and
build workflow and adds `validate_modpack_plan` before final confirmation.

Tools:

- Existing base-pack search, inspection, mod search, detail, resolution,
  build, presentation, and instance-list tools.
- `validate_modpack_plan`.

`build_modpack` also runs validation internally and returns a blocked manifest
when the plan has blocking issues. Prompt compliance is not a safety boundary.

### Instance entry

The instance entry is bound to one `root + instance_id` by the desktop host.
The model cannot supply or change that binding.

Tools:

- Existing `wiki_search` and `wiki_open`.
- Provider search, detail, and dependency resolution tools for requested
  instance changes.
- `diagnose_instance`.
- `show_instance_changes`.

The instance prompt covers gameplay questions, diagnosis, and maintenance. It
must use Wiki evidence for pack content and diagnostic/provider evidence for
compatibility claims.

## Shared Compatibility Report

Rust owns a serializable report contract:

```rust
pub struct CompatibilityReport {
    pub status: CompatibilityStatus,
    pub issues: Vec<CompatibilityIssue>,
}

pub struct CompatibilityIssue {
    pub code: String,
    pub severity: CompatibilitySeverity,
    pub summary: String,
    pub subjects: Vec<String>,
    pub evidence: Vec<String>,
    pub suggested_actions: Vec<SuggestedAction>,
}
```

Status is `healthy`, `warning`, or `blocked`. Severity is `info`, `warning`, or
`blocking`. Codes are stable machine-readable identifiers; summaries and
evidence are human-readable but must not contain account data or unrestricted
local paths.

The first implementation recognizes these issue codes:

- `last_launch_crash`
- `duplicate_mod_id`
- `mod_loader_mismatch`
- `memory_below_recommendation`
- `selected_version_incompatible`
- `missing_required_dependency`
- `declared_mod_conflict`
- `duplicate_project`

The report is extensible: later Java, graphics, Mixin, sandbox, and richer jar
dependency analysis can add codes without changing the tool contracts.

## `diagnose_instance`

The model supplies no path or instance ID. The desktop injects the bound root
and instance ID before IPC.

Input:

```ts
{
  include_log_tail?: boolean;
}
```

Output contains:

- Instance name, Minecraft version, loader, configured memory, and Mod count.
- The shared `CompatibilityReport`.
- An optional bounded log tail when requested.

Checks:

1. Read the instance summary and configuration.
2. Scan enabled local Mod metadata.
3. Flag duplicate enabled `mod_id` values.
4. Flag clear loader-family mismatches. Quilt accepts Fabric Mods; unknown
   metadata does not produce a mismatch claim.
5. Compare configured memory with the existing deterministic recommendation.
6. Read a bounded tail from the instance's `logs/latest.log`, falling back to
   the newest crash report.
7. Run the existing crash analyzer and add its category, evidence line, and
   suggestions as a structured issue.

Missing logs are a healthy no-data condition, not an error. Unreadable or
invalid instance configuration remains an actionable tool error.

## `validate_modpack_plan`

Input reuses the build target, optional base pack, and exact resolved extra-Mod
references used by `build_modpack`. The tool does not write files.

Validation:

1. Normalize provider/project keys and reject duplicate selected projects.
2. Re-fetch every exact selected version from its provider.
3. Verify the selected version supports the target Minecraft version and
   loader and has a downloadable primary file.
4. Read the base pack's project list when a base is present.
5. Build required and incompatible dependency edges from provider metadata.
6. Treat a required dependency as satisfied only when its project is in the
   selected extras or base project list.
7. Report a declared incompatibility as blocking only when the incompatible
   target is actually selected or present in the base.

Unresolved exact versions, incompatible target versions, missing required
dependencies, and actual declared conflicts are blocking. Duplicate project
selection is blocking. Provider/network failures remain tool errors so the
model can retry rather than misreporting a compatibility conclusion.

`build_modpack` invokes the same validator before writing. A blocked report is
returned in the existing execution manifest shape with `status: "blocked"`, a
stable reason, and the full compatibility report.

## `show_instance_changes`

This is a client-side interactive tool, analogous to `show_modpack`. It pauses
for a user click and never mutates the instance merely because the model called
it.

Initial operations:

```ts
type InstanceChange =
  | { kind: "set_memory"; memory_mb: number }
  | { kind: "set_mod_enabled"; file_name: string; enabled: boolean }
  | { kind: "delete_mod"; file_name: string }
  | {
      kind: "install_mod";
      provider: "modrinth" | "curseforge";
      project_id: string;
    };
```

The card displays each action and its reason. Confirm executes actions in order
through existing desktop commands. Skip performs no writes. The result reports
completed actions and the first failure; it never claims later actions ran
after a failure.

Safety rules:

- The bound instance context is host-injected.
- File names must match values returned by instance diagnosis/listing and are
  validated again by existing core functions.
- Memory is bounded by the same UI/core constraints.
- Mod installation derives Minecraft version and loader from the bound
  instance, not model input.
- Delete continues to use the existing trash-first implementation.
- User confirmation is mandatory for every change card.

## Prompt Changes

The build prompt requires this sequence for customized builds:

```text
resolve -> validate -> present final plan -> explicit user confirmation
-> build -> show
```

The instance prompt requires:

- Gameplay and recipe questions: Wiki tools.
- Crash, launch, compatibility, or performance questions: diagnose first.
- Requested changes: diagnose or inspect, search/resolve when needed, then
  `show_instance_changes`.
- Never treat a proposed or skipped change as applied.

Both prompts keep real provider IDs, version IDs, paths, and outcomes grounded
in tool results.

## Desktop And Runtime Integration

- `AgentMode` becomes `"build" | "instance"`; persisted `"modpack"` and
  `"wiki"` values migrate to `"build"` and `"instance"` respectively.
- `buildTools(entry)` returns the complete relevant tool set for that entry.
- Both OpenRouter and local Claude Code use the same entry-specific prompt and
  tool names.
- The desktop dispatcher injects instance context for instance tools and
  rejects them when no bound instance exists.
- `show_instance_changes` joins the existing interactive-tool pause channel.

## Testing

Deterministic Rust tests cover:

- Duplicate Mod IDs and loader mismatch diagnosis.
- Missing logs and bounded log-tail behavior.
- Existing crash-rule translation into compatibility issues.
- Exact-version target compatibility.
- Required dependency satisfaction by selected extras or base Mods.
- Declared conflicts block only when the target is present.
- `build_modpack` refuses a blocked plan before writing output.
- Change execution input validation through existing core operations.

TypeScript tests cover:

- Entry-specific tool sets and prompts.
- Host-owned instance parameters cannot appear in model schemas.
- `show_instance_changes` schema validation.
- Dispatcher mode/context guards.
- Interactive tool pause and result handling.

Lightweight fixture evals cover:

- Build requests call validation before build.
- Crash questions call diagnosis instead of Wiki search.
- Recipe questions remain on Wiki tools.
- Instance changes are shown for confirmation rather than executed directly.

## Non-Goals

- No progressive tool disclosure or `activate_tools` layer.
- No arbitrary shell or filesystem tools.
- No automatic sandbox clone/test/promote loop yet.
- No exhaustive static compatibility proof for undocumented runtime conflicts.
- No automatic mutation without a user-confirmed card.

## Success Criteria

- Build and instance entry behavior remains distinct and explicit.
- Both entries reuse one structured compatibility report.
- A bound instance can be diagnosed without model-supplied paths.
- Customized builds cannot proceed past known blocking dependency conflicts.
- Instance changes reuse existing commands and require a user click.
- Existing build and Wiki workflows continue to pass their regression tests.
