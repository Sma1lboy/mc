const TOOL_NAMES = [
  "search_base_modpacks",
  "inspect_base_modpack",
  "search_mods",
  "mod_get_detail",
  "resolve_mods",
  "build_modpack",
  "list_instances",
  "wiki_search",
  "wiki_open",
  "ask_user_question",
  "show_modpack",
];

const VALID_MODES = new Set(["modpack", "wiki"]);

/**
 * Pure session router for the line-delimited Claude host protocol.
 * Process IO is intentionally injected so concurrency and identity routing are
 * deterministic under test.
 */
export function createHarnessHostRouter({ send, createAgent, model }) {
  const sessions = new Map();

  function modeFrom(value) {
    return VALID_MODES.has(value) ? value : "modpack";
  }

  function sendDone(conversationId, runId, error) {
    send({
      type: "done",
      conversationId,
      runId,
      ...(error ? { error } : {}),
    });
  }

  function createSession(conversationId, mode) {
    const session = {
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
    );
    sessions.set(conversationId, session);
    return session;
  }

  async function sessionFor(conversationId, mode) {
    const current = sessions.get(conversationId);
    if (!current || current.mode === mode) return current ?? createSession(conversationId, mode);
    await current.agent.dispose();
    return createSession(conversationId, mode);
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
        conversationId: session.conversationId,
        runId,
        toolCallId,
        name,
        args,
      });
    });
  }

  async function runTurn(message) {
    const conversationId = String(message.conversationId ?? "");
    const runId = String(message.runId ?? "");
    if (!conversationId || !runId) return;
    const mode = modeFrom(message.mode);
    const existing = sessions.get(conversationId);
    if (existing?.running) {
      sendDone(conversationId, runId, "turn already running");
      return;
    }
    const session = await sessionFor(conversationId, mode);
    if (session.running) {
      sendDone(conversationId, runId, "turn already running");
      return;
    }

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
    try {
      const result = await session.agent.run(
        session.history,
        (assistant) =>
          send({ type: "update", conversationId, runId, message: assistant }),
        session.abort.signal,
      );
      session.history = result.messages;
      sendDone(conversationId, runId, result.error);
    } catch (error) {
      sendDone(conversationId, runId, error instanceof Error ? error.message : String(error));
    } finally {
      if (session.activeRunId === runId) {
        session.running = false;
        session.activeRunId = null;
        session.abort = null;
      }
    }
  }

  function resolveTool(message) {
    const conversationId = String(message.conversationId ?? "");
    const runId = String(message.runId ?? "");
    const toolCallId = String(message.toolCallId ?? "");
    const session = sessions.get(conversationId);
    const key = `${runId}\u0000${toolCallId}`;
    const pending = session?.pendingTools.get(key);
    if (!pending) return;
    session.pendingTools.delete(key);
    if (message.ok) pending.resolve(message.result);
    else pending.reject(new Error(String(message.error ?? "tool failed")));
  }

  function abortTurn(message) {
    const session = sessions.get(String(message.conversationId ?? ""));
    if (session?.activeRunId === String(message.runId ?? "")) session.abort?.abort();
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
  }

  return { handle, dispose };
}
