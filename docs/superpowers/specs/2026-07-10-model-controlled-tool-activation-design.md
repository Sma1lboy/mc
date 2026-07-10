# Model-Controlled Tool Activation

Date: 2026-07-10

## Summary

kobeMC will expose one user-facing modpack assistant instead of separate
`modpack` and `wiki` assistants. The model will decide which deferred kobeMC
tools it needs by calling an always-available `activate_tools` tool. The host
will apply the requested active-tool set on the next model step while retaining
the same conversation and runtime session.

This design separates two concerns:

- The model owns semantic routing: which tools are useful for the task.
- The host owns authority: which requested tools are valid in the current
  context and whether an operation requires user approval.

This is the foundation for adding instance diagnosis, build-plan conflict
validation, instance modification, and sandbox experiments without creating a
new assistant mode for each capability.

## Scope

The first implementation covers model-controlled activation for the existing
agent tool catalog across both runtime engines:

- OpenRouter through AI SDK `ToolLoopAgent`.
- Local Claude Code through `HarnessAgent` and its persistent session.
- Desktop conversation persistence and client-tool dispatch.
- A unified system prompt with a concise deferred-tool directory.
- Migration of existing `modpack` and `wiki` conversations.

The following capabilities are intentionally deferred to follow-up specs:

- `diagnose_instance` and the shared `CompatibilityReport` engine.
- `validate_modpack_plan` and mandatory build preflight checks.
- Instance mutation and approval cards.
- Disposable-instance sandbox experiments and test-launch loops.

## Product Model

There is one assistant identity. A conversation may optionally be bound to an
installed instance through host-owned context. Binding an instance changes
which tool requests the host can approve, but it does not switch prompts or
assistant identities.

The model starts with a small bootstrap surface and activates deferred tools as
needed. Activated tools remain active for that conversation until the model
replaces the active set or the conversation is reset.

## Tool Catalog

### Always-active tools

- `activate_tools`: add or replace deferred tools for the conversation.
- `ask_user_question`: present a bounded choice when user input is required.

`activate_tools` cannot activate runtime builtins such as shell, file read, or
file write. Its schema is generated only from the kobeMC deferred-tool
registry.

### Deferred tools

The existing kobeMC tools become deferred:

- `search_base_modpacks`
- `inspect_base_modpack`
- `search_mods`
- `mod_get_detail`
- `resolve_mods`
- `build_modpack`
- `show_modpack`
- `list_instances`
- `wiki_search`
- `wiki_open`

Future diagnostic and instance-management tools will register in the same
catalog without introducing a new agent mode.

### Catalog metadata

Each deferred tool has host-owned metadata in addition to its AI SDK schema:

```ts
interface DeferredToolMetadata {
  name: DeferredToolName;
  summary: string;
  requiresInstance: boolean;
  interaction: "automatic" | "user-confirmed";
}
```

The unified prompt contains only each tool's name and short summary. Full tool
descriptions and input schemas are supplied to the model only while the tool is
active.

## `activate_tools` Contract

Input:

```ts
{
  tools: DeferredToolName[];
  mode?: "add" | "replace";
}
```

`mode` defaults to `add`. `replace` replaces only deferred tools; always-active
tools cannot be removed.

Output:

```ts
{
  active_tools: DeferredToolName[];
  added: DeferredToolName[];
  removed: DeferredToolName[];
  denied: Array<{
    tool: DeferredToolName;
    code: "INSTANCE_CONTEXT_REQUIRED" | "TOOL_UNAVAILABLE";
    message: string;
  }>;
  catalog_version: number;
}
```

The model may request several related tools in one call. Activation is
idempotent. Unknown tool names fail schema validation and never reach the host.

## Activation State

The desktop owns the canonical activation state:

```ts
interface AgentToolState {
  activeTools: DeferredToolName[];
  catalogVersion: number;
}
```

It is stored with `ConversationRecord`, restored when switching conversations,
and reset for a new conversation. Tool names are sorted and deduplicated before
storage so runtime cache keys remain stable.

Old conversations migrate as follows:

- Legacy `wiki` context starts with `wiki_search` and `wiki_open` active.
- Legacy `modpack` context starts with its previous modpack tools active.
- New conversations start with no deferred tools active.

The legacy mode field remains readable during migration but is not used to
select the system prompt.

## Runtime Flow

### Shared flow

```text
user request
  -> model sees bootstrap tools and active deferred tools
  -> model calls activate_tools when another tool is needed
  -> host validates the request and updates conversation tool state
  -> activation result is appended to the same UIMessage history
  -> model resumes with the updated active-tool definitions
  -> model calls the selected domain tool
```

Activation is an internal orchestration event. It remains in model history for
correct tool-call semantics but is not rendered as a visible chat activity.

### OpenRouter engine

The OpenRouter agent is configured with the complete kobeMC tool registry and
an `activeTools` list containing always-active tools plus the conversation's
activated deferred tools.

`activate_tools` remains a client-side tool. After the desktop writes its
result, the existing `drive()` continuation calls the agent again with the same
`UIMessage[]`. The runtime configuration is rebuilt from the updated active
set, while `convertToModelMessages` continues to receive the complete registry
so historical tool calls remain parseable.

### Local Claude Code engine

`HarnessAgent` filtering is construction-time, but `HarnessAgentSession` is an
explicit object that is not owned by one agent definition. The local host will
therefore:

1. Keep one persistent `HarnessAgentSession` per conversation.
2. Configure `activate_tools` as an approval-interrupted custom tool, while the
   desktop handles that approval automatically and never presents it as a user
   decision.
3. Read and validate the requested names from the paused tool call.
4. Construct a new `HarnessAgent` definition with the updated `activeTools`.
5. Continue the suspended turn on the same session, approve the activation
   call, and return the validated activation result to the model.

This preserves Claude Code conversation state and avoids exposing all custom
tools merely to work around construction-time filtering. Builtin coding tools
remain excluded from every active set.

## Prompt Rules

The unified prompt must state:

- Select tools based on the user's task, not the page that opened the chat.
- Call `activate_tools` before calling a deferred tool that is not active.
- Activate the smallest useful set, but activate a complete short workflow in
  one call when the next steps are already clear.
- Use Wiki tools only for local gameplay and pack-content evidence.
- Use provider and compatibility tools for dependency or runtime claims.
- Treat a denied activation as a context limitation and explain what the user
  must open or select before retrying.
- Never claim a write, install, repair, or launch occurred without a successful
  tool result from the current conversation.

Tool descriptions remain self-contained and continue to state what the tool
does, when to use it, what it returns, and how to recover from errors.

## Authorization And Safety

Activation is not authorization.

- The host intersects requested names with the kobeMC deferred-tool registry.
- Instance-bound tools require host-injected instance context.
- The dispatcher rejects calls to inactive tools even if a model emits one.
- Interactive and future mutating tools retain their own user-approval gates.
- `activate_tools` cannot enable harness builtin tools.
- Paths, instance IDs, account data, and output roots remain host-owned.
- Mandatory deterministic validation inside mutating operations cannot be
  disabled through activation.

If a model emits `activate_tools` in parallel with inactive domain tools, the
host processes activation and returns `TOOL_NOT_ACTIVE` for the premature
domain calls. The prompt tells the model to retry them after activation.

## Errors

Agent-visible errors use stable codes and actionable messages:

- `TOOL_NOT_ACTIVE`: call `activate_tools` with the named tool, then retry.
- `INSTANCE_CONTEXT_REQUIRED`: open or select an installed instance first.
- `TOOL_UNAVAILABLE`: the requested tool is not available in this host build.
- `CATALOG_VERSION_MISMATCH`: refresh activation state from the host result.

Errors never reveal local paths or credentials.

## Compatibility

The public `ModpackAgent.run(history, onUpdate, signal)` contract remains
unchanged. Runtime-specific activation details stay behind the agent adapters.

`AgentMode` is retained temporarily as a legacy persisted-data type, but new
runtime creation does not branch prompts or tool catalogs on it. The desktop
passes instance context and activation state independently.

## Testing

### Deterministic unit tests

- `activate_tools` accepts known deferred names and rejects unknown names.
- Add mode is idempotent and replace mode preserves always-active tools.
- Instance-bound tools are denied without instance context.
- The dispatcher rejects inactive calls even if the payload is valid.
- Conversation serialization restores a stable, deduplicated active set.
- Legacy `wiki` and `modpack` records migrate to their prior effective tools.
- Historical tool calls still convert after their tools become inactive.

### Runtime contract tests

- OpenRouter: activation changes the next continuation's active definitions.
- Local harness: activation continues on the same session with a new filtered
  Agent definition.
- Both engines keep builtin shell/read/write tools inactive.
- Switching conversations cannot leak activated tools between conversations.

### Lightweight agent evals

Fixture-driven cases verify routing behavior:

- A recipe question activates `wiki_search` and does not activate build tools.
- A vague new-pack request activates discovery tools.
- Adding a specific mod activates search, resolve, and build tools.
- A Wiki request without instance context handles the denied activation.
- The model does not claim success from an activation result alone.

## Rollout

1. Add registry metadata, activation schema, and unit tests.
2. Add desktop activation state, persistence, migration, and dispatcher guard.
3. Integrate OpenRouter `activeTools` continuation.
4. Integrate Harness session continuation with dynamic Agent definitions.
5. Merge prompts and remove runtime mode branching.
6. Add lightweight routing evals and run both runtime contract suites.
7. Register diagnostic and compatibility tools in a follow-up change.

## Success Criteria

- The model can activate and use an initially deferred tool without a new user
  message.
- OpenRouter and local Claude Code preserve conversation state through
  activation.
- Inactive tool schemas are not sent to either model runtime.
- Activation state is isolated and persisted per conversation.
- Host context and approval checks remain authoritative.
- Existing modpack-building and instance-Wiki workflows continue to work after
  automatic activation.
- The activation mechanism has no visible UI beyond the domain tool activity
  that follows it.
