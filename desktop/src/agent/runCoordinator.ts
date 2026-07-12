import type { UIMessage } from "ai";

export interface AgentProviderSession {
  run(request: AgentProviderRunRequest): Promise<{ messages: UIMessage[]; error?: string }>;
}

export interface AgentRunBinding {
  readonly conversationId: string;
  readonly runId: string;
  readonly providerSession: AgentProviderSession;
  readonly toolContext: unknown;
  readonly abortController: AbortController;
}

export interface AgentProviderRunRequest {
  binding: AgentRunBinding;
  history: UIMessage[];
  onUpdate: (assistant: UIMessage) => void;
  signal: AbortSignal;
}

export interface ConversationRunState {
  id: string;
  messages: UIMessage[];
  queued: string[];
  error: string | null;
  streaming: boolean;
  toolContext: unknown;
  pendingInteractiveToolCallIds: string[];
}

type RunStatus = "active" | "completed" | "cancelled" | "superseded";

interface RunRecord {
  binding: AgentRunBinding;
  status: RunStatus;
}

interface InteractiveResolver {
  conversationId: string;
  runId: string;
  toolCallId: string;
  resolve: (output: unknown) => void;
  reject: (error: Error) => void;
}

interface CoordinatorOptions {
  createProviderSession: (input: {
    conversationId: string;
    toolContext: unknown;
  }) => Promise<AgentProviderSession>;
  isAutomaticTool: (name: string) => boolean;
  isInteractiveTool: (name: string) => boolean;
  runAutomaticTool: (name: string, input: unknown, toolContext: unknown) => Promise<unknown>;
  onChange: (state: ConversationRunState) => void;
  onSelectedChange?: (state: ConversationRunState) => void;
  makeRunId: () => string;
  makeMessageId: () => string;
}

export class AgentRunCoordinator {
  private readonly conversations = new Map<string, ConversationRunState>();
  private readonly providerSessions = new Map<string, Promise<AgentProviderSession>>();
  private readonly latestRuns = new Map<string, RunRecord>();
  private readonly interactiveResolvers = new Map<string, InteractiveResolver>();
  private selectedId: string | null = null;

  constructor(private readonly options: CoordinatorOptions) {}

  openConversation(
    id: string,
    seed: { messages: UIMessage[]; toolContext: unknown },
  ): ConversationRunState {
    const existing = this.conversations.get(id);
    if (existing) return this.snapshot(existing);
    const state: ConversationRunState = {
      id,
      messages: seed.messages.slice(),
      queued: [],
      error: null,
      streaming: false,
      toolContext: seed.toolContext,
      pendingInteractiveToolCallIds: [],
    };
    this.conversations.set(id, state);
    this.emit(state);
    return this.snapshot(state);
  }

  getConversation(id: string): ConversationRunState {
    return this.snapshot(this.requireConversation(id));
  }

  selectConversation(id: string): void {
    const state = this.requireConversation(id);
    this.selectedId = id;
    this.options.onSelectedChange?.(this.snapshot(state));
  }

  selectedConversationId(): string | null {
    return this.selectedId;
  }

  setToolContext(conversationId: string, toolContext: unknown): void {
    const state = this.requireConversation(conversationId);
    state.toolContext = toolContext;
    this.emit(state);
  }

  clearProviderSessions(): void {
    this.providerSessions.clear();
  }

  async sendMessage(conversationId: string, raw: string): Promise<void> {
    const text = raw.trim();
    if (!text) return;
    const state = this.requireConversation(conversationId);
    if (state.streaming) {
      state.queued.push(text);
      this.emit(state);
      return;
    }
    await this.runQueuedTurns(conversationId, text);
  }

  enqueueMessage(conversationId: string, raw: string): void {
    const text = raw.trim();
    if (!text) return;
    const state = this.requireConversation(conversationId);
    state.queued.push(text);
    this.emit(state);
  }

  dequeueMessage(conversationId: string, index: number): void {
    const state = this.requireConversation(conversationId);
    state.queued = state.queued.filter((_, current) => current !== index);
    this.emit(state);
  }

  cancelConversation(conversationId: string): void {
    const run = this.latestRuns.get(conversationId);
    if (!run || run.status !== "active") return;
    run.status = "cancelled";
    run.binding.abortController.abort();
    this.rejectInteractiveForRun(run, new Error("agent run cancelled"));
    const state = this.requireConversation(conversationId);
    this.emit(state);
  }

  waitForInteractiveTool(
    binding: AgentRunBinding,
    _name: string,
    toolCallId: string,
  ): Promise<unknown> {
    const run = this.latestRuns.get(binding.conversationId);
    if (!run || run.binding.runId !== binding.runId || run.status !== "active") {
      return Promise.reject(new Error("agent run is no longer active"));
    }
    const key = interactiveKey(binding.conversationId, binding.runId, toolCallId);
    if (this.interactiveResolvers.has(key)) {
      return Promise.reject(new Error(`interactive tool already pending: ${toolCallId}`));
    }
    const state = this.requireConversation(binding.conversationId);
    state.pendingInteractiveToolCallIds.push(toolCallId);
    this.emit(state);
    return new Promise((resolve, reject) => {
      this.interactiveResolvers.set(key, {
        conversationId: binding.conversationId,
        runId: binding.runId,
        toolCallId,
        resolve,
        reject,
      });
    });
  }

  resolveInteractiveTool(
    conversationId: string,
    runId: string,
    toolCallId: string,
    output: unknown,
  ): boolean {
    const key = interactiveKey(conversationId, runId, toolCallId);
    const pending = this.interactiveResolvers.get(key);
    if (!pending) return false;
    this.interactiveResolvers.delete(key);
    const state = this.requireConversation(conversationId);
    state.pendingInteractiveToolCallIds = state.pendingInteractiveToolCallIds.filter(
      (id) => id !== toolCallId,
    );
    this.emit(state);
    pending.resolve(output);
    return true;
  }

  resolveInteractiveToolForConversation(
    conversationId: string,
    messageId: string,
    toolCallId: string,
    output: unknown,
  ): boolean {
    const pending = [...this.interactiveResolvers.values()].find(
      (entry) => entry.conversationId === conversationId && entry.toolCallId === toolCallId,
    );
    if (!pending) return false;
    const state = this.requireConversation(conversationId);
    state.messages = setToolOutput(state.messages, messageId, toolCallId, output);
    return this.resolveInteractiveTool(conversationId, pending.runId, toolCallId, output);
  }

  async continueConversation(conversationId: string, messages: UIMessage[]): Promise<void> {
    const state = this.requireConversation(conversationId);
    if (state.streaming) return;
    state.messages = messages.slice();
    await this.runHistory(conversationId, state.messages);
  }

  private async runQueuedTurns(conversationId: string, firstText: string): Promise<void> {
    let text: string | undefined = firstText;
    while (text) {
      await this.runOneTurn(conversationId, text);
      const state = this.requireConversation(conversationId);
      text = state.queued.shift();
      if (text) this.emit(state);
    }
  }

  private async runOneTurn(conversationId: string, text: string): Promise<void> {
    const state = this.requireConversation(conversationId);
    const user: UIMessage = {
      id: this.options.makeMessageId(),
      role: "user",
      parts: [{ type: "text", text }],
    };
    const history = [...state.messages, user];
    await this.runHistory(conversationId, history);
  }

  private async runHistory(conversationId: string, initialHistory: UIMessage[]): Promise<void> {
    const state = this.requireConversation(conversationId);
    let history = initialHistory;
    state.messages = history;
    state.streaming = true;
    state.error = null;
    this.emit(state);

    let run: RunRecord | null = null;
    try {
      const providerSession = await this.providerSession(conversationId, state.toolContext);
      const previous = this.latestRuns.get(conversationId);
      if (previous) previous.status = "superseded";
      const abortController = new AbortController();
      const binding: AgentRunBinding = Object.freeze({
        conversationId,
        runId: this.options.makeRunId(),
        providerSession,
        toolContext: cloneAndFreeze(state.toolContext),
        abortController,
      });
      const activeRun: RunRecord = { binding, status: "active" };
      run = activeRun;
      this.latestRuns.set(conversationId, activeRun);
      while (activeRun.status === "active") {
        const inputHistory = history;
        const result = await providerSession.run({
          binding,
          history: inputHistory,
          signal: abortController.signal,
          onUpdate: (assistant) => {
            if (!canRouteEvent(activeRun.status)) return;
            state.messages = [...inputHistory, assistant];
            this.emit(state);
          },
        });
        if (activeRun.status !== "active") break;
        history = result.messages;
        state.messages = history;
        state.error = result.error ?? null;
        this.emit(state);
        if (result.error) break;
        const resolved = await this.resolveAutomaticTools(activeRun, history);
        if (!resolved) break;
        history = resolved.messages;
        state.messages = history;
        this.emit(state);
        if (!resolved.shouldResume) break;
      }
    } catch (error) {
      if (!run || run.status === "active") {
        state.error = error instanceof Error ? error.message : String(error);
      }
      if (!run) this.providerSessions.delete(conversationId);
    } finally {
      if (run?.status === "active") run.status = "completed";
      if (!run || this.latestRuns.get(conversationId) === run) {
        state.streaming = false;
        this.emit(state);
      }
    }
  }

  private async resolveAutomaticTools(
    run: RunRecord,
    messages: UIMessage[],
  ): Promise<{ messages: UIMessage[]; shouldResume: boolean } | null> {
    const assistant = [...messages].reverse().find((entry) => entry.role === "assistant");
    if (!assistant) return null;
    const pending = assistant.parts
      .filter(isToolPart)
      .filter((part) => part.state === "input-available")
      .map((part) => ({ part, name: toolName(part) }))
      .filter(({ name }) => this.options.isAutomaticTool(name));
    if (pending.length === 0) return null;

    let next = messages;
    for (const { part, name } of pending) {
      if (run.status !== "active") return null;
      try {
        const output = await this.options.runAutomaticTool(
          name,
          part.input,
          run.binding.toolContext,
        );
        next = setToolOutput(next, assistant.id, part.toolCallId, output);
      } catch (error) {
        next = setToolError(
          next,
          assistant.id,
          part.toolCallId,
          error instanceof Error ? error.message : String(error),
        );
      }
    }
    const pendingInteractive = assistant.parts
      .filter(isToolPart)
      .some(
        (part) =>
          part.state === "input-available" && this.options.isInteractiveTool(toolName(part)),
      );
    return { messages: next, shouldResume: !pendingInteractive };
  }

  private providerSession(
    conversationId: string,
    toolContext: unknown,
  ): Promise<AgentProviderSession> {
    let session = this.providerSessions.get(conversationId);
    if (!session) {
      session = this.options.createProviderSession({ conversationId, toolContext });
      this.providerSessions.set(conversationId, session);
    }
    return session;
  }

  private rejectInteractiveForRun(run: RunRecord, error: Error): void {
    for (const [key, pending] of this.interactiveResolvers) {
      if (
        pending.conversationId !== run.binding.conversationId ||
        pending.runId !== run.binding.runId
      ) {
        continue;
      }
      this.interactiveResolvers.delete(key);
      pending.reject(error);
    }
    const state = this.requireConversation(run.binding.conversationId);
    state.pendingInteractiveToolCallIds = [];
  }

  private requireConversation(id: string): ConversationRunState {
    const state = this.conversations.get(id);
    if (!state) throw new Error(`unknown conversation: ${id}`);
    return state;
  }

  private emit(state: ConversationRunState): void {
    this.options.onChange(this.snapshot(state));
  }

  private snapshot(state: ConversationRunState): ConversationRunState {
    return {
      ...state,
      messages: state.messages.slice(),
      queued: state.queued.slice(),
      pendingInteractiveToolCallIds: state.pendingInteractiveToolCallIds.slice(),
    };
  }
}

function interactiveKey(conversationId: string, runId: string, toolCallId: string): string {
  return `${conversationId}\u0000${runId}\u0000${toolCallId}`;
}

function canRouteEvent(status: RunStatus): boolean {
  return status !== "cancelled" && status !== "superseded";
}

function cloneAndFreeze<T>(value: T): T {
  if (value == null || typeof value !== "object") return value;
  const clone = structuredClone(value);
  return deepFreeze(clone);
}

function deepFreeze<T>(value: T): T {
  if (value == null || typeof value !== "object" || Object.isFrozen(value)) return value;
  for (const child of Object.values(value)) deepFreeze(child);
  return Object.freeze(value);
}

function isToolPart(
  part: UIMessage["parts"][number],
): part is Extract<UIMessage["parts"][number], { toolCallId: string }> & { input?: unknown } {
  return typeof (part as { toolCallId?: unknown }).toolCallId === "string";
}

function toolName(part: { type: string }): string {
  return part.type.startsWith("tool-") ? part.type.slice("tool-".length) : "";
}

function setToolOutput(
  messages: UIMessage[],
  messageId: string,
  toolCallId: string,
  output: unknown,
): UIMessage[] {
  return updateToolPart(messages, messageId, toolCallId, (part) => ({
    ...part,
    state: "output-available",
    output,
  }));
}

function setToolError(
  messages: UIMessage[],
  messageId: string,
  toolCallId: string,
  errorText: string,
): UIMessage[] {
  return updateToolPart(messages, messageId, toolCallId, (part) => ({
    ...part,
    state: "output-error",
    errorText,
  }));
}

function updateToolPart(
  messages: UIMessage[],
  messageId: string,
  toolCallId: string,
  update: (part: Extract<UIMessage["parts"][number], { toolCallId: string }>) => unknown,
): UIMessage[] {
  return messages.map((entry) => {
    if (entry.id !== messageId) return entry;
    return {
      ...entry,
      parts: entry.parts.map((part) =>
        isToolPart(part) && part.toolCallId === toolCallId ? update(part) : part,
      ),
    } as UIMessage;
  });
}
