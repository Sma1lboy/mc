# Agent Action Approval Boundary Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent the model from directly triggering modpack builds or visible deep-diagnosis launches by moving both operations behind launcher-rendered confirmation tools.

**Architecture:** The model may propose `confirm_modpack_build` or `confirm_deep_diagnosis`, both client-side tools with no `execute`. Their cards render the exact action and only invoke the existing Tauri command from a user click. The raw `build_modpack` and `start_deep_diagnosis` tools leave the model profiles and automatic dispatcher, while Rust remains the deterministic executor.

**Tech Stack:** TypeScript, React, Zustand, Vercel AI SDK tool schemas, Tauri IPC, Vitest.

## Global Constraints

- This worktree implements P0-1 only; conversation/run isolation belongs to the separate P0-2 task.
- Merge the P0-2 run-isolation PR first (or rebase this PR after it) so local-runtime interactive resolvers are correlated by `toolCallId` before these additional confirmation tools ship.
- Raw privileged operations must not remain in the model tool registry or automatic dispatcher.
- Build and deep-diagnosis cancellation must return a tool result and resume the same conversation without executing IPC.
- The OpenRouter and Claude Code runtime profiles must expose the same approval tools.
- No new runtime dependencies.

---

### Task 1: Lock the model-facing boundary with failing tests

**Files:**
- Modify: `packages/agent-core/test/agent.test.ts`
- Create: `desktop/src/agent/clientToolDispatcher.test.ts`

**Interfaces:**
- Consumes: `buildTools`, `toolSchemasForMode`, `isAutomaticClientTool`, `runLauncherClientTool`.
- Produces: Regression expectations that raw privileged tools are absent and confirmation tools are interactive.

- [ ] Add tests asserting build mode exposes `confirm_modpack_build` but not `build_modpack`.
- [ ] Add tests asserting instance mode exposes `confirm_deep_diagnosis` but not `start_deep_diagnosis`.
- [ ] Add dispatcher tests asserting neither raw operation is automatic and direct dispatch rejects both names.
- [ ] Run the focused tests and confirm they fail because the old raw tools are still exposed.

### Task 2: Replace raw tools with approval schemas

**Files:**
- Create: `packages/agent-core/src/tools/confirm-modpack-build.ts`
- Modify: `packages/agent-core/src/tools/deep-diagnosis.ts`
- Modify: `packages/agent-core/src/tools/index.ts`
- Modify: `packages/agent-core/src/prompt.ts`
- Modify: `packages/agent-core/src/harness/index.ts`
- Modify: `packages/agent-core/bin/harness-host.mjs`

**Interfaces:**
- Produces: `CONFIRM_MODPACK_BUILD_TOOL`, `CONFIRM_DEEP_DIAGNOSIS_TOOL`, client-side schemas for the final build plan and diagnosis reason.
- Consumes: Existing modpack plan schema and local-runtime interactive handler bridge.

- [ ] Implement `confirm_modpack_build` with the exact existing build arguments and no execute function.
- [ ] Replace `start_deep_diagnosis` in the instance model profile with `confirm_deep_diagnosis({ reason })`.
- [ ] Update prompts so the card itself is the confirmation request and its output drives the next step.
- [ ] Update the Claude host interactive tool bridge and fallback list.
- [ ] Run agent-core tests and confirm the model-facing boundary tests pass.

### Task 3: Render and execute the approved build action

**Files:**
- Create: `desktop/src/agent/BuildConfirmationCard.tsx`
- Modify: `desktop/src/agent/ChatParts.tsx`
- Modify: `desktop/src/agent/clientToolDispatcher.ts`
- Modify: `desktop/src/agent/chatStore.ts`
- Modify: `desktop/src/locales/agent.ts`

**Interfaces:**
- Consumes: `commands.agentToolBuildModpack`, `resolveClientTool`, model-supplied exact plan.
- Produces: Build confirmation card returning either the build result or `{ approved: false }`.

- [ ] Add a component-level testable action helper that invokes no IPC when declined and exactly one IPC when approved.
- [ ] Render the exact MC version, loader, base, extra-mod count, and output filename.
- [ ] Keep the card actionable while the local runtime is paused on this interactive tool.
- [ ] Remove `build_modpack` from automatic dispatch and reject direct dispatch.

### Task 4: Render and execute the approved deep-diagnosis action

**Files:**
- Create: `desktop/src/agent/DeepDiagnosisConfirmationCard.tsx`
- Modify: `desktop/src/agent/ChatParts.tsx`
- Modify: `desktop/src/agent/clientToolDispatcher.ts`
- Modify: `desktop/src/agent/chatStore.ts`
- Modify: `desktop/src/locales/agent.ts`

**Interfaces:**
- Consumes: Bound `AgentInstanceContext`, `commands.agentToolStartDeepDiagnosis`, `resolveClientTool`.
- Produces: Diagnosis confirmation card returning either the native start result or `{ approved: false }`.

- [ ] Require a bound instance context before enabling approval.
- [ ] Show the visible-launch, installed-Mod execution, offline-mode, trial-limit, and non-security-sandbox boundaries.
- [ ] Invoke start exactly once on approval and never on decline.
- [ ] Remove `start_deep_diagnosis` from automatic dispatch and reject direct dispatch.

### Task 5: Verify the complete P0-1 boundary

**Files:**
- Test: `packages/agent-core/test/agent.test.ts`
- Test: `desktop/src/agent/clientToolDispatcher.test.ts`
- Test: relevant desktop component/helper tests.

**Interfaces:**
- Produces: Evidence that the model cannot directly reach either privileged command.

- [ ] Run `npm test --workspace @kobemc/agent-core`.
- [ ] Run the desktop Vitest files through `npx vitest run`.
- [ ] Run `npx tsc --noEmit -p desktop/tsconfig.json`.
- [ ] Run `npm run build --prefix desktop`.
- [ ] Review `git diff --check`, `git status --short`, and the scoped diff.
