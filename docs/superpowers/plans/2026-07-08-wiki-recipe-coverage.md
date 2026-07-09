# Wiki Recipe Coverage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expand the local wiki agent's structured gameplay knowledge beyond shaped crafting recipes.

**Architecture:** Reuse the existing `WikiSourceDocument` and recipe `structured` payload in `crates/mc-core/src/agent/tools/wiki.rs`. Keep the wire shape compatible with the current `recipe_card` renderer by improving `result`, `ingredients`, and `grid` extraction rather than adding a new UI contract.

**Tech Stack:** Rust `mc-core`, serde_json, existing zip/test fixtures, cargo test.

## Global Constraints

- Keep launcher logic in `mc-core`; do not move wiki parsing into Tauri.
- Do not add dependencies.
- Use TDD and commit each independently useful behavior.
- Preserve local source precedence: KubeJS/datapack recipe data outranks mod jar defaults.

---

### Task 1: Non-Crafting JSON Recipes

**Files:**
- Modify: `crates/mc-core/src/agent/tools/tests.rs`
- Modify: `crates/mc-core/src/agent/tools/wiki.rs`

**Interfaces:**
- Consumes: `tool_wiki_search(WikiSearchArgs)` and existing `recipe_document_from_json`.
- Produces: structured `kind: "recipe"` hits for smelting-like, stonecutting, and smithing recipes with usable `result` and `ingredients`.

- [ ] **Step 1: Write the failing tests**

Add tests that build a fake mod jar with `minecraft:smelting`, `minecraft:stonecutting`, and `minecraft:smithing_transform` recipes, then assert `kind == "recipe"`, exact `target_id` lookup works, and `ingredient_id` lookup finds the source ingredient.

- [ ] **Step 2: Run tests to verify red**

Run: `cargo test -p mc-core wiki_search_indexes_non_crafting_recipe_json -- --nocapture`

Expected: FAIL because smithing/stonecutting ingredient fields are not fully normalized yet.

- [ ] **Step 3: Implement minimal parser expansion**

Extend recipe extraction helpers in `wiki.rs` to normalize common recipe input keys: `ingredient`, `ingredients`, `input`, `base`, `addition`, `template`, and keyed shaped `key`. Preserve existing shaped grid behavior.

- [ ] **Step 4: Run tests to verify green**

Run: `cargo test -p mc-core wiki_search_indexes_non_crafting_recipe_json -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/mc-core/src/agent/tools/tests.rs crates/mc-core/src/agent/tools/wiki.rs
git commit -m "feat(agent): index non-crafting wiki recipes"
```

### Task 2: KubeJS Helper Recipes

**Files:**
- Modify: `crates/mc-core/src/agent/tools/tests.rs`
- Modify: `crates/mc-core/src/agent/tools/wiki.rs`

**Interfaces:**
- Consumes: `kubejs_recipe_documents_from_script` and existing JS argument helpers.
- Produces: structured `kind: "recipe"` hits for `event.shaped(...)`, `event.shapeless(...)`, and common processing helpers.

- [ ] **Step 1: Write the failing tests**

Add a KubeJS server script fixture using `event.shaped`, `event.shapeless`, and `event.smelting`, then assert `target_id` and `ingredient_id` searches find those generated recipe documents.

- [ ] **Step 2: Run tests to verify red**

Run: `cargo test -p mc-core wiki_search_indexes_kubejs_helper_recipes -- --nocapture`

Expected: FAIL because only `event.custom(...)` currently emits recipe documents.

- [ ] **Step 3: Implement minimal parser expansion**

Add small adapters from the common KubeJS helper call arguments into vanilla-style recipe JSON strings, then pass them through `recipe_document_from_json`.

- [ ] **Step 4: Run tests to verify green**

Run: `cargo test -p mc-core wiki_search_indexes_kubejs_helper_recipes -- --nocapture`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/mc-core/src/agent/tools/tests.rs crates/mc-core/src/agent/tools/wiki.rs
git commit -m "feat(agent): index kubejs helper wiki recipes"
```

### Task 3: Final Verification

**Files:**
- No new files.

**Interfaces:**
- Consumes: all tests from Tasks 1 and 2.
- Produces: verified branch ready for review.

- [ ] **Step 1: Run focused wiki tests**

Run: `cargo test -p mc-core wiki_search_`

Expected: PASS.

- [ ] **Step 2: Run agent-core schema/render tests if prompt or UI changed**

Run only if TypeScript files changed: `npm test -w packages/agent-core -- agent.test.ts`

Expected: PASS.

- [ ] **Step 3: Inspect final diff**

Run: `git status --short && git diff --stat HEAD`

Expected: only planned files changed.
