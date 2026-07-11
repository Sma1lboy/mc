# Deep Instance Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bounded sandbox launches that let the instance agent test allowlisted remediation hypotheses without modifying the installed instance.

**Architecture:** Rust owns snapshot creation, operation validation, session budgets, launch execution, log capture, and cleanup. Agent-core exposes three non-overlapping stateful tool contracts; the desktop injects the bound instance context and dispatches them. Successful trials can only be promoted by calling the existing user-confirmed `show_instance_changes` tool.

**Tech Stack:** Rust 2021, Tokio, serde, Tauri v2, specta, TypeScript, Zod, AI SDK, Cargo tests, Vitest.

## Global Constraints

- Do not expose source roots, instance ids, arbitrary paths, shell commands, file contents, JVM arguments, scripts, source edits, or JAR edits to the model.
- Trial operations are limited to memory changes, Mod enable/disable, and sandbox-only Mod deletion.
- Every trial starts from a clean baseline snapshot and uses a synthetic offline identity.
- The host owns timeout, session lifetime, and trial-count limits.
- Deep diagnosis never writes back to the installed instance.
- Real-instance remediation continues to require `show_instance_changes` and a user click.
- The diagnostic sandbox is not presented as an OS or hostile-code security boundary.

---

### Task 1: Snapshot And Trial Operation Core

**Files:**
- Create: `crates/mc-core/src/agent/tools/deep_diagnosis.rs`
- Modify: `crates/mc-core/src/agent/tools/mod.rs`
- Modify: `crates/mc-core/src/agent/tools/tests.rs`

**Interfaces:**
- Produces: `DiagnosticTrialOperation`, `DiagnosticSandboxSnapshot`, `create_diagnostic_snapshot`, `prepare_diagnostic_trial`, and `cleanup_diagnostic_session`.
- Consumes: `GamePaths`, `Instance`, `InstanceConfig`, and the existing Mod enabled-file convention.

- [ ] Add failing tests that prove snapshots exclude user/runtime directories and symlinks and never write under the source root.
- [ ] Run `cargo test -p mc-core diagnostic_snapshot_` and verify the missing API fails.
- [ ] Implement bounded regular-file copying, read-only launch-input reuse, opaque session directories, and cleanup.
- [ ] Add failing tests for valid memory/enable/delete operations plus traversal, nested path, unknown file, install-like, and excessive-operation rejection.
- [ ] Implement operation validation and apply operations only below the trial's copied instance directory.
- [ ] Run `cargo test -p mc-core deep_diagnosis` and keep all focused tests green.

### Task 2: Stateful Bounded Launch Commands

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Regenerate: `desktop/src/ipc/bindings.ts`

**Interfaces:**
- Produces Tauri commands `agent_tool_start_deep_diagnosis`, `agent_tool_run_diagnostic_trial`, and `agent_tool_finish_deep_diagnosis`.
- Session state binds every opaque id to one host-injected source root and instance id, with a maximum of three hypothesis trials and a thirty-minute lifetime.
- Launch output reports `stable`, `crashed`, or `launch_error`, exit code, elapsed time, bounded logs, deterministic crash analysis, and the exact operations tested.

- [ ] Add failing pure-state tests for unknown/cross-instance sessions, trial budgets, expiry cleanup, and idempotent finish.
- [ ] Implement the session registry and baseline/trial preparation around Task 1 APIs.
- [ ] Implement a fixed sixty-second offline launch with synthetic credentials, cleared server auto-join, bounded pipe draining, explicit kill-and-wait, and structured outcome classification.
- [ ] Register commands, regenerate bindings with `cargo test --manifest-path desktop/src-tauri/Cargo.toml export_bindings`, and run `cargo check --manifest-path desktop/src-tauri/Cargo.toml`.

### Task 3: Agent Contracts, Routing, And Evaluation

**Files:**
- Create: `packages/agent-core/src/tools/deep-diagnosis.ts`
- Modify: `packages/agent-core/src/tools/index.ts`
- Modify: `packages/agent-core/src/prompt.ts`
- Modify: `packages/agent-core/test/agent.test.ts`
- Modify: `desktop/src/agent/clientToolDispatcher.ts`
- Modify: `desktop/src/agent/clientToolDispatcher.test.ts`
- Modify: `scripts/instance-agent-eval.mjs`

**Interfaces:**
- Model inputs contain only an opaque `session_id` and the strict diagnostic operation union; source context remains host-injected.
- The instance profile receives all three deep tools. Build mode receives none.

- [ ] Add failing schema/tool-set tests proving forbidden mutation fields and host-owned context cannot be supplied.
- [ ] Implement the three Zod tools and desktop dispatch with mode and bound-context guards.
- [ ] Update the instance prompt to require static diagnosis and explicit user approval before a sandbox launch, independent trials, finish/cleanup, and confirmation-card-only promotion.
- [ ] Add eval cases for deep routing, forbidden code/JAR edits, and successful-trial promotion through `show_instance_changes`.
- [ ] Run agent-core tests, dispatcher tests, TypeScript checks, instance evals, Cargo tests, and desktop build.
