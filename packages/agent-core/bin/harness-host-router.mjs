const TOOL_NAMES = [
  "search_base_modpacks",
  "inspect_base_modpack",
  "search_mods",
  "mod_get_detail",
  "resolve_mods",
  "validate_modpack_plan",
  "confirm_modpack_build",
  "list_instances",
  "wiki_search",
  "wiki_open",
  "diagnose_instance",
  "confirm_deep_diagnosis",
  "run_diagnostic_trial",
  "finish_deep_diagnosis",
  "ask_user_question",
  "show_modpack",
  "show_instance_changes",
];

const VALID_MODES = new Map([
  ["build", "build"],
  ["modpack", "build"],
  ["instance", "instance"],
  ["wiki", "instance"],
]);

/**
 * Pure session router for the line-delimited Claude host protocol.
 * Provider session ids are epochs: returning to Claude after another provider
 * must create fresh Claude history for that conversation.
 */
export function createHarnessHostRouter({ send, createAgent, model }) {
  const sessions = new Map(); // providerSessionId -> session
  const busyConversations = new Set();

  function modeFrom(value) {
    return VALID_MODES.get(value) ?? "build";
  }

  function sendDone(providerSessionId, conversationId, runId, error, promptVersion) {
    send({
      type: "done",
      providerSessionId,
      conversationId,
      runId,
      ...(error ? { error } : {}),
      ...(promptVersion ? { promptVersion } : {}),
    });
  }

  function createSession(providerSessionId, conversationId, mode) {
    const session = {
      providerSessionId,
      conversationId,
      mode,
      history: [],
      uid: 0,
      agent: null,
      running: false,
      activeRunId: null,
      abort: null,
      pendingTools: new Map(),
    };
    const handlers = Object.fromEntries(
      TOOL_NAMES.map((name) => [
        name,
        (args, options = {}) => callTool(session, name, args, options.toolCallId),
      ]),
    );
    session.agent = createAgent(
      handlers,
      { ...(model ? { model } : {}), mode },
      conversationId,
      providerSessionId,
    );
    sessions.set(providerSessionId, session);
    return session;
  }

  async function sessionFor(providerSessionId, conversationId, mode) {
    const current = sessions.get(providerSessionId);
    if (current && current.conversationId !== conversationId) {
      throw new Error("provider session belongs to another conversation");
    }
    if (current?.mode === mode) return current;
    if (current) {
      await current.agent.dispose();
      sessions.delete(providerSessionId);
    }
    for (const [id, other] of sessions) {
      if (other.conversationId !== conversationId || other.running) continue;
      await other.agent.dispose();
      sessions.delete(id);
    }
    return createSession(providerSessionId, conversationId, mode);
  }

  function callTool(session, name, args, toolCallId) {
    const runId = session.activeRunId;
    if (!session.running || !runId) {
      return Promise.reject(new Error("tool call has no active run"));
    }
    if (typeof toolCallId !== "string" || !toolCallId) {
      return Promise.reject(new Error("tool call id is required"));
    }
    const key = `${runId}\u0000${toolCallId}`;
    if (session.pendingTools.has(key)) {
      return Promise.reject(new Error(`tool call already pending: ${toolCallId}`));
    }
    return new Promise((resolve, reject) => {
      session.pendingTools.set(key, { resolve, reject });
      send({
        type: "tool_call",
        providerSessionId: session.providerSessionId,
        conversationId: session.conversationId,
        runId,
        toolCallId,
        name,
        args,
      });
    });
  }

  async function runTurn(message) {
    const providerSessionId = String(message.providerSessionId ?? "");
    const conversationId = String(message.conversationId ?? "");
    const runId = String(message.runId ?? "");
    if (!providerSessionId || !conversationId || !runId) return;
    if (busyConversations.has(conversationId)) {
      sendDone(providerSessionId, conversationId, runId, "turn already running");
      return;
    }
    busyConversations.add(conversationId);
    let session;
    try {
      session = await sessionFor(providerSessionId, conversationId, modeFrom(message.mode));
      session.running = true;
      session.activeRunId = runId;
      session.abort = new AbortController();
      session.history = [
        ...session.history,
        {
          id: `u${++session.uid}`,
          role: "user",
          parts: [{ type: "text", text: String(message.text ?? "") }],
        },
      ];
      const result = await session.agent.run(
        session.history,
        (assistant) =>
          send({
            type: "update",
            providerSessionId,
            conversationId,
            runId,
            message: assistant,
          }),
        session.abort.signal,
      );
      session.history = result.messages;
      sendDone(providerSessionId, conversationId, runId, result.error, result.promptVersion);
    } catch (error) {
      sendDone(
        providerSessionId,
        conversationId,
        runId,
        error instanceof Error ? error.message : String(error),
      );
    } finally {
      if (session?.activeRunId === runId) {
        session.running = false;
        session.activeRunId = null;
        session.abort = null;
      }
      busyConversations.delete(conversationId);
    }
  }

  function resolveTool(message) {
    const providerSessionId = String(message.providerSessionId ?? "");
    const conversationId = String(message.conversationId ?? "");
    const runId = String(message.runId ?? "");
    const toolCallId = String(message.toolCallId ?? "");
    const session = sessions.get(providerSessionId);
    if (session?.conversationId !== conversationId) return;
    const key = `${runId}\u0000${toolCallId}`;
    const pending = session.pendingTools.get(key);
    if (!pending) return;
    session.pendingTools.delete(key);
    if (message.ok) pending.resolve(message.result);
    else pending.reject(new Error(String(message.error ?? "tool failed")));
  }

  function abortTurn(message) {
    const session = sessions.get(String(message.providerSessionId ?? ""));
    if (
      session?.conversationId === String(message.conversationId ?? "") &&
      session.activeRunId === String(message.runId ?? "")
    ) {
      session.abort?.abort();
    }
  }

  function handle(message) {
    switch (message?.type) {
      case "turn":
        void runTurn(message);
        break;
      case "tool_result":
        resolveTool(message);
        break;
      case "abort":
        abortTurn(message);
        break;
    }
  }

  async function dispose() {
    await Promise.all([...sessions.values()].map((session) => session.agent.dispose()));
    sessions.clear();
    busyConversations.clear();
  }

  return { handle, dispose };
}
