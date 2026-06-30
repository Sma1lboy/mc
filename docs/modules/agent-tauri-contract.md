# Agent Tauri Contract

This document describes the intended UI/daemon contract for the local modpack
agent. The CLI is only a harness. Tauri should render agent state directly and
send structured actions for button clicks instead of routing every click through
natural language.

## Core Model

The daemon-facing state object is `AgentRunSnapshot` from `mc-core`.

Important fields for UI:

| Field | Purpose |
|------|---------|
| `id` | Stable session id. |
| `status` | `running`, `waiting_for_user`, `completed`, or `failed`. |
| `phase` | Workflow phase. Drives which UI panel is shown. |
| `messages` | Audit/user-visible messages. Not a complete chat response contract. |
| `pending_approval` | Current HITL gate, if the agent is waiting for user input. |
| `plan` | Human-readable plan summary for the current phase. |
| `restrictions` | Normalized Minecraft/loader/content requirements. |
| `mod_plan` | In-progress reducer state for extra mod planning. Mostly diagnostic. |
| `approved_build` | Final approved build contract used by execution. |
| `execution` | Execution status/manifest/blocking reason. |
| `trace` | Internal audit/debug trace. Do not render as primary UI. |

The frontend should treat snapshots as the single source of truth. Re-render from
the latest snapshot after every command.

## Approval Gate

When `status == waiting_for_user`, render `pending_approval`.

`ApprovalRequest` fields:

| Field | Purpose |
|------|---------|
| `id` | Gate instance id. Send it back with structured actions to prevent stale clicks. |
| `kind` | Gate type: requirements, base-pack selection, customization, etc. |
| `title` | Short UI title. |
| `message` | Gate-specific user guidance. |
| `options` | Selectable options. Render as cards/list rows/buttons. |
| `available_decisions` | Allowed decisions and whether they need option/message. |
| `plan` | Optional phase plan summary. |

`ApprovalOption` fields:

| Field | Purpose |
|------|---------|
| `id` | Stable option id for the current gate. |
| `label` | UI label. |
| `description` | Optional secondary text. |
| `payload` | Structured backing data. Use for details panels; send only `id` back. |

## Recommended Tauri Commands

The command layer can be thin. It should load the snapshot from
`AgentSessionStore`, call `MainAgentRuntime`, save the returned snapshot, and
return a frontend response object.

Recommended command shape:

```rust
#[tauri::command]
async fn agent_start(
    prompt: String,
    entry: AgentEntry,
    session_id: Option<String>,
) -> Result<AgentUiResponse>;

#[tauri::command]
async fn agent_get(session_id: String) -> Result<AgentUiResponse>;

#[tauri::command]
async fn agent_list() -> Result<Vec<AgentSessionSummary>>;

#[tauri::command]
async fn agent_delete(session_id: String) -> Result<bool>;

#[tauri::command]
async fn agent_action(session_id: String, action: AgentClientAction) -> Result<AgentUiResponse>;

#[tauri::command]
async fn agent_execute_export(session_id: String, output_path: String) -> Result<AgentUiResponse>;
```

`AgentEntry` is supplied by the frontend route that opened the agent UI. The
model must not infer this from the user prompt.

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEntry {
    Home,
}
```

The daemon resolves `AgentEntry` into an `AgentLaunchContext` and injects the
available workflows. The frontend should not send `available_workflows`.

| Entry | Subject | Injected workflows |
| --- | --- | --- |
| `home` | none | `build_modpack` |

Unsupported or unavailable intents should return a completed unsupported
snapshot instead of silently falling through to another workflow.

`AgentUiResponse` is a frontend-facing wrapper around the snapshot:

```rust
pub struct AgentUiResponse {
    pub snapshot: AgentRunSnapshot,
    pub assistant_response: String,
    pub actions: Vec<AgentUiAction>,
}
```

`assistant_response` should be deterministically rendered from snapshot state.
Do not require an extra LLM call just to write UI copy.

## Structured Actions

Button clicks should use structured actions. Do not convert buttons into natural
language such as "choose the first option" and send them through approval
routing.

Recommended action enum:

```rust
pub enum AgentClientAction {
    Approve {
        approval_id: String,
        option_id: Option<String>,
    },
    ChooseOption {
        approval_id: String,
        option_id: String,
    },
    Revise {
        approval_id: String,
        message: String,
    },
    Cancel {
        approval_id: String,
    },
    ContinueText {
        message: String,
    },
}
```

Mapping to core decisions:

| UI action | Core decision |
|----------|---------------|
| `Approve { option_id }` | `UserDecisionKind::Approve` with `selected_option_id`. |
| `ChooseOption { option_id }` | Same as approve, but clearer for option cards. |
| `Revise { message }` | `UserDecisionKind::Revise` with message. |
| `Cancel` | `UserDecisionKind::Cancel`. |
| `ContinueText` | Natural-language fallback; runtime may route through LLM. |

The Tauri layer should reject stale actions when `approval_id` does not match
`snapshot.pending_approval.id`.

If `MainAgentRuntime` only exposes a natural-language continue entrypoint, add a
thin core method that accepts an already-structured `UserDecision`. This keeps
button clicks deterministic and avoids unnecessary model calls.

## Phase Rendering

Suggested UI panels:

| Phase | UI |
|------|----|
| `ConfigureRequirementsApproval` | Requirements summary, missing fields, confirm/revise/cancel. |
| `ChooseBasePackApproval` | Base-pack option cards plus "Start from scratch". |
| `CustomizationPlanning` | Progress/loading state. The reducer may call provider APIs and LLM. |
| `ConfirmCustomizationApproval` | Final mod list, unresolved requests, confirm/revise/back. |
| `ExecutionReady` | Export destination picker and explicit "Export .mrpack" action. |
| `Executing` / `Verifying` | Progress/status. |
| `Completed` | Artifact path and summary. |
| `Failed` | Error and recovery options. |

## Assistant Response Renderer

The current `messages` array is audit-oriented and not guaranteed to contain a
polished response for every turn. The frontend should render an
`assistant_response` from phase/state.

Examples:

| State | Response pattern |
|------|------------------|
| Requirements gate | "I understood the target as `<loader> / <version>` with tags `<tags>`. Please confirm or revise." |
| Missing requirements | "I still need `<missing fields>` before I can search safely." |
| Base-pack gate | "I found these base-pack candidates. Pick one or start from scratch." |
| Customization gate | "I prepared a compatible mod plan. Review the additions and unresolved requests before exporting." |
| Execution ready | "The plan is approved. Choose an output path to export the `.mrpack`." |
| Completed | "Export completed: `<path>`." |

## Unresolved Requests

At customization approval, check:

```text
pending_approval.options[id == "confirm:recommended_customization"]
  .payload.validation.unresolved_goals
```

Each unresolved goal contains:

| Field | Purpose |
|------|---------|
| `goal_id` | Internal goal id. |
| `label` | User-facing request text. |
| `status` | Usually `open`. |
| `diagnosis` | Explanation from the reducer/model. |
| `next_step` | Suggested recovery. |

Render these prominently. They are the user-visible explanation for requests
that were not added, such as a requested mod being unavailable for the current
version/loader.

## Execution

Do not execute automatically from a normal continue action.

Flow:

1. User approves customization.
2. Snapshot becomes `status=running`, `phase=ExecutionReady`,
   `execution.status=NotStarted`.
3. UI asks for/export path.
4. UI calls `agent_execute_export(session_id, output_path)`.
5. The daemon calls deterministic `advance`/execution.
6. Return updated snapshot.

If the session is not executable, return a user-facing error such as:

```text
This session does not have an approved executable plan yet. Complete the approval gates first.
```

If a session is already completed and the user chooses a new output path, either
copy the recorded artifact or clearly report where the artifact already exists.

## Error Handling

Do not leak internal snapshot file paths to the UI. Missing sessions should be
reported as:

```text
Session '<id>' was not found.
```

Invalid text at a gate should leave state unchanged and return the same
approval gate with a clarification message.

## Do Not

- Do not render `trace` as primary UI.
- Do not treat `messages` as the final chat-response contract.
- Do not route button clicks through LLM text routing.
- Do not execute/export from `continue`.
- Do not trust LLM-generated URLs, hashes, filenames, or env metadata. Execution
  data must come from provider APIs and deterministic packaging.

## Known Gaps

- Version-flexible base-pack discovery is not implemented. Requests like
  "version does not matter; find a Terraria-like modpack" still need a separate
  workflow change.
- Mod planning quality is still improving. Some baseline/search goals can select
  unrelated projects; render unresolved and allow revise.
- Long-running agent operations are currently snapshot-returning calls. A future
  daemon API should add job ids, progress events, cancellation, and retry.
