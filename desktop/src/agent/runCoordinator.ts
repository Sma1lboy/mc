import type { UIMessage } from "ai";

export interface AgentProviderSession {
  run(
    request: AgentProviderRunRequest,
  ): Promise<{ messages: UIMessage[]; error?: string; promptVersion?: string }>;
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
  waitingInteractive: boolean;
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
  private readonly interactiveRunOwners = new Map<string, string>();
  private readonly interactiveMessageOwners = new Map<string, string>();
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
      waitingInteractive: hasPendingInteractiveTool(seed.messages, this.options.isInteractiveTool),
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
    if (state.streaming || state.waitingInteractive) {
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
    this.interactiveRunOwners.set(
      interactiveOwnerKey(binding.conversationId, toolCallId),
      binding.runId,
    );
    this.bindInteractiveMessageOwners(binding.conversationId, binding.runId, toolCallId);
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
    const ownerKey = interactiveOwnerKey(conversationId, toolCallId);
    if (this.interactiveRunOwners.get(ownerKey) === runId) {
      this.interactiveRunOwners.delete(ownerKey);
    }
    for (const [messageOwnerKey, ownerRunId] of this.interactiveMessageOwners) {
      if (
        ownerRunId === runId &&
        messageOwnerKey.startsWith(`${conversationId}\u0000`) &&
        messageOwnerKey.endsWith(`\u0000${toolCallId}`)
      ) {
        this.interactiveMessageOwners.delete(messageOwnerKey);
      }
    }
    const state = this.requireConversation(conversationId);
    state.pendingInteractiveToolCallIds = state.pendingInteractiveToolCallIds.filter(
      (id) => id !== toolCallId,
    );
    this.emit(state);
    pending.resolve(output);
    return true;
  }

  interactiveToolRunId(
    conversationId: string,
    messageId: string,
    toolCallId: string,
  ): string | undefined {
    return this.interactiveMessageOwners.get(
      interactiveMessageOwnerKey(conversationId, messageId, toolCallId),
    );
  }

  resolveClientToolOutput(
    conversationId: string,
    messageId: string,
    toolCallId: string,
    output: unknown,
    expectedRunId?: string,
  ): "local" | "waiting" | "resume" | "ignored" {
    const pending = expectedRunId
      ? this.interactiveResolvers.get(interactiveKey(conversationId, expectedRunId, toolCallId))
      : [...this.interactiveResolvers.values()].find(
          (entry) => entry.conversationId === conversationId && entry.toolCallId === toolCallId,
        );
    if (expectedRunId && !pending) return "ignored";
    const state = this.requireConversation(conversationId);
    if (!hasToolCall(state.messages, messageId, toolCallId)) return "ignored";
    state.messages = setToolOutput(state.messages, messageId, toolCallId, output);
    if (pending) {
      this.resolveInteractiveTool(conversationId, pending.runId, toolCallId, output);
      return "local";
    }
    state.waitingInteractive = hasPendingInteractiveTool(
      state.messages,
      this.options.isInteractiveTool,
    );
    this.emit(state);
    return state.waitingInteractive ? "waiting" : "resume";
  }

  async continueConversation(conversationId: string, messages: UIMessage[]): Promise<void> {
    const state = this.requireConversation(conversationId);
    if (state.streaming) return;
    state.waitingInteractive = false;
    state.messages = messages.slice();
    await this.runHistory(conversationId, state.messages);
    await this.drainQueuedTurns(conversationId);
  }

  private async runQueuedTurns(conversationId: string, firstText: string): Promise<void> {
    await this.runOneTurn(conversationId, firstText);
    await this.drainQueuedTurns(conversationId);
  }

  private async drainQueuedTurns(conversationId: string): Promise<void> {
    const state = this.requireConversation(conversationId);
    while (!state.streaming && !state.waitingInteractive && state.queued.length > 0) {
      const text = state.queued.shift();
      this.emit(state);
      if (text) await this.runOneTurn(conversationId, text);
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
    state.waitingInteractive = false;
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
            this.bindInteractiveMessageOwnersFromMessage(activeRun, assistant);
            this.emit(state);
          },
        });
        if (activeRun.status !== "active") break;
        history = result.messages;
        state.messages = history;
        state.error = result.error ?? null;
        this.emit(state);
        if (result.error) break;
        const processed = await this.processClientTools(activeRun, history);
        history = processed.messages;
        state.messages = history;
        this.emit(state);
        if (processed.action === "resume") continue;
        if (processed.action === "wait") state.waitingInteractive = true;
        break;
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

  private async processClientTools(
    run: RunRecord,
    messages: UIMessage[],
  ): Promise<{ messages: UIMessage[]; action: "done" | "resume" | "wait" }> {
    const assistant = [...messages].reverse().find((entry) => entry.role === "assistant");
    if (!assistant) return { messages, action: "done" };
    const pending = assistant.parts
      .filter(isToolPart)
      .filter((part) => part.state === "input-available")
      .map((part) => ({ part, name: toolName(part) }))
      .filter(({ name }) => this.options.isAutomaticTool(name));
    let next = messages;
    for (const { part, name } of pending) {
      if (run.status !== "active") return { messages: next, action: "done" };
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
    if (hasPendingInteractiveTool(next, this.options.isInteractiveTool)) {
      return { messages: next, action: "wait" };
    }
    return { messages: next, action: pending.length > 0 ? "resume" : "done" };
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

  private bindInteractiveMessageOwners(
    conversationId: string,
    runId: string,
    toolCallId: string,
  ): void {
    const state = this.requireConversation(conversationId);
    for (const message of [...state.messages].reverse()) {
      if (
        message.parts.some(
          (part) => isToolPart(part) && part.toolCallId === toolCallId,
        )
      ) {
        this.interactiveMessageOwners.set(
          interactiveMessageOwnerKey(conversationId, message.id, toolCallId),
          runId,
        );
        break;
      }
    }
  }

  private bindInteractiveMessageOwnersFromMessage(run: RunRecord, message: UIMessage): void {
    for (const part of message.parts) {
      if (!isToolPart(part)) continue;
      const owner = this.interactiveRunOwners.get(
        interactiveOwnerKey(run.binding.conversationId, part.toolCallId),
      );
      if (owner !== run.binding.runId) continue;
      this.interactiveMessageOwners.set(
        interactiveMessageOwnerKey(
          run.binding.conversationId,
          message.id,
          part.toolCallId,
        ),
        run.binding.runId,
      );
    }
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

function interactiveOwnerKey(conversationId: string, toolCallId: string): string {
  return `${conversationId}\u0000${toolCallId}`;
}

function interactiveMessageOwnerKey(
  conversationId: string,
  messageId: string,
  toolCallId: string,
): string {
  return `${conversationId}\u0000${messageId}\u0000${toolCallId}`;
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

function hasToolCall(messages: UIMessage[], messageId: string, toolCallId: string): boolean {
  return messages.some(
    (message) =>
      message.id === messageId &&
      message.parts.some((part) => isToolPart(part) && part.toolCallId === toolCallId),
  );
}

function hasPendingInteractiveTool(
  messages: UIMessage[],
  isInteractiveTool: (name: string) => boolean,
): boolean {
  const assistant = [...messages].reverse().find((message) => message.role === "assistant");
  return (
    assistant?.parts
      .filter(isToolPart)
      .some(
        (part) => part.state === "input-available" && isInteractiveTool(toolName(part)),
      ) ?? false
  );
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
