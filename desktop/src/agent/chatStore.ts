// 整合包助手聊天 store —— zustand(单一真相,任何组件 useChatStore 即读,任何模块 action 即写)。
// ------------------------------------------------------------------
// 大脑是 TS(@kobemc/agent-core,在 webview 内跑 AI SDK 的 tool-use loop);Rust 聊天大脑已删除。
//
// 会话就是一个 `UIMessage[]`(AI SDK 原生消息 —— 单一真相:既渲染又喂模型)。发送 / 续跑都走
// `agent.run(history, onUpdate)`:onUpdate 每次给出「正在生长的助手 UIMessage」,直接替换到列表尾;
// 文本 / 思考 / 工具调用(input-streaming → available → output 状态机)全由 AI SDK 累积,不再手写归约器。
//
// 所有 agent-core tools 都是原生 client-side tools(无 execute):模型调用后 tool part 停在
// input-available(无 output)。自动工具由本 store 通过 IPC 调 Rust 并写回 output;交互工具
// (ask_user_question / show_modpack)等用户点击后写回 output。随后再 run 一次续跑同一会话。
// ------------------------------------------------------------------

import { create } from "zustand";
import type { UIMessage } from "ai";
// Type-only import: erased at build, so the host-agnostic brain (and its `ai`
// dependency) stays out of the main bundle — the TS path is dynamic-imported below.
import { setCurrentPage, kobeUser, useAppStore, activeRoot } from "../store";
import { commands } from "../ipc/bindings";
import { t } from "../i18n";
import {
  INTERACTIVE_CLIENT_TOOLS,
  isAutomaticClientTool,
  runLauncherClientTool,
} from "./clientToolDispatcher";
import {
  agentModeFromContext,
  type AgentToolContext,
} from "./agentContext";
import {
  AgentRunCoordinator,
  type AgentProviderSession,
  type ConversationRunState,
} from "./runCoordinator";

export type { AgentToolContext, AgentWikiContext } from "./agentContext";

interface ChatState {
  /** 当前 UI 投影对应的对话；异步动作必须捕获并显式回传此 id。 */
  conversationId: string;
  /** 会话消息(AI SDK 原生 UIMessage:含 text / reasoning / tool parts)。 */
  messages: UIMessage[];
  /** 是否正在流式(显示指示器)。同一会话同一时刻只允许一轮。 */
  streaming: boolean;
  /**
   * 排队待发的用户消息(Claude Code 式):流式期间发送即入队,本轮结束后按 FIFO 依次各自成一轮。
   */
  queued: string[];
  /** 本轮失败原因(流式外呈现;UIMessage 本身无 error part)。 */
  error: string | null;
  /**
   * 一次性输入草稿:外部入口(发现页 / 新建实例)经 openAgentChat 预填一句上下文提示,
   * ChatPage 变为活动页后取用一次(填进输入框、聚焦,不自动发送),随即清回 null。
   */
  draft: string | null;
  /** 历史对话记录(dev 调试选择器用;localStorage 持久)。 */
  conversations: ConversationRecord[];
  /** Host-owned deterministic tool context, never supplied by the model. */
  toolContext: AgentToolContext | null;
  /**
   * 本地引擎(claude-code)模式下正在等用户作答的交互 toolCallId 集合。
   * 此时 turn 仍在流式,
   * 但对应组件必须可交互 —— 见 AskUserOptions / ModpackCard 的 live 判定。
   */
  pendingLocalToolCallIds: string[];
}

export const useChatStore = create<ChatState>(() => ({
  conversationId: "",
  messages: [],
  streaming: false,
  queued: [],
  error: null,
  draft: null,
  conversations: loadConversations(),
  toolContext: null,
  pendingLocalToolCallIds: [],
}));

async function createProviderSession(input: {
  conversationId: string;
  toolContext: unknown;
}): Promise<AgentProviderSession> {
  const toolContext = input.toolContext as AgentToolContext | null;
  const mode = agentModeFromContext(toolContext);
  const settings = await commands.getSettings();
  const provider =
    settings.status === "ok" ? (settings.data.agent_provider ?? "openrouter") : "openrouter";
  if (provider === "claude-code") {
    const m = await import("./localRuntimeAdapter");
    return m.createLocalRuntimeAgent(mode, {
      waitForInteractiveTool: (binding, name, toolCallId) =>
        coordinator.waitForInteractiveTool(binding, name, toolCallId),
    });
  }
  const m = await import("./desktopAdapter");
  return m.createDesktopAgent(mode);
}

// 稳定的自增 id(仅前端展示 key;convertToModelMessages 会丢弃 UIMessage.id)。
let seq = 0;
const nextId = (): string => `${Date.now().toString(36)}-${(seq++).toString(36)}`;

// 会话 id:一次「对话」一个,newChat 时轮换。
const mintConvId = (): string =>
  `chat-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
let currentConvId = mintConvId();

/** 当前会话 id。dev 调试面板复制它,或按 id 回溯整轮 flow。 */
export function currentChatSessionId(): string {
  return currentConvId;
}

/* ——— 会话记录 ———
 * 每轮结束把当前对话存到 localStorage(离线缓存),登录 kobeMC 账号后同时同步到
 * mc-server 数据库(按 updatedAt 新者胜,跨设备保留);DebugTools 据此列出、切换。
 * 记录就是 UIMessage[](既渲染又能喂模型),切换会话即无缝续聊。 */
export interface ConversationRecord {
  id: string;
  createdAt: number;
  updatedAt: number;
  /** 首条用户消息(截断),用作列表标题。 */
  title: string;
  messages: UIMessage[];
  toolContext?: AgentToolContext | null;
}

const CONV_KEY = "mc-launcher.agentConversations";
const CONV_LIMIT = 50;

function loadConversations(): ConversationRecord[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(CONV_KEY);
    const list = raw ? (JSON.parse(raw) as ConversationRecord[]) : [];
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

function persistConversations(list: ConversationRecord[]): void {
  try {
    window.localStorage.setItem(CONV_KEY, JSON.stringify(list.slice(0, CONV_LIMIT)));
  } catch {
    /* WebView 里 localStorage 可能不可用 */
  }
}

/** 首条用户消息的纯文本(会话标题用)。 */
function firstUserText(messages: UIMessage[]): string {
  const first = messages.find((m) => m.role === "user");
  if (!first) return "";
  return first.parts
    .map((p) => (p.type === "text" ? p.text : ""))
    .join("")
    .trim();
}

function projectConversation(state: ConversationRunState): void {
  useChatStore.setState({
    conversationId: state.id,
    messages: state.messages,
    streaming: state.streaming,
    queued: state.queued,
    error: state.error,
    toolContext: state.toolContext as AgentToolContext | null,
    pendingLocalToolCallIds: state.pendingInteractiveToolCallIds,
  });
}

let runSeq = 0;
const coordinator = new AgentRunCoordinator({
  createProviderSession,
  isAutomaticTool: isAutomaticClientTool,
  isInteractiveTool: (name) => INTERACTIVE_CLIENT_TOOLS.has(name),
  runAutomaticTool: (name, input, context) =>
    runLauncherClientTool(name, input, context as AgentToolContext | null),
  onChange: (state) => {
    if (state.id === currentConvId) projectConversation(state);
  },
  onSelectedChange: projectConversation,
  makeRunId: () => `run-${Date.now().toString(36)}-${(runSeq++).toString(36)}`,
  makeMessageId: nextId,
});

function contextWithRoot(context: AgentToolContext | null): AgentToolContext {
  return { ...(context ?? {}), root: context?.root ?? activeRoot() };
}

coordinator.openConversation(currentConvId, {
  messages: [],
  toolContext: contextWithRoot(null),
});
coordinator.selectConversation(currentConvId);

/** Invalidate sessions used by future runs without touching already-bound runs. */
export function resetAgent(): void {
  coordinator.clearProviderSessions();
}

// 把指定对话 upsert 进记录列表。空对话不存。
function saveConversation(conversationId: string): void {
  const runtime = coordinator.getConversation(conversationId);
  const messages = runtime.messages;
  if (messages.length === 0) return;
  const now = Date.now();
  const list = useChatStore.getState().conversations.slice();
  const i = list.findIndex((c) => c.id === conversationId);
  const createdAt = i >= 0 ? list[i].createdAt : now;
  const rec: ConversationRecord = {
    id: conversationId,
    createdAt,
    updatedAt: now,
    title: firstUserText(messages).slice(0, 60),
    messages,
    toolContext: runtime.toolContext as AgentToolContext | null,
  };
  if (i >= 0) list[i] = rec;
  else list.unshift(rec);
  persistConversations(list);
  useChatStore.setState({ conversations: list });
  pushConversation(rec);
}

/* ——— 云端同步 ———
 * 登录后每次存档都 fire-and-forget 推到 mc-server(agent_conversations 表,按账号);
 * syncConversations() 双向合并:两侧都以记录自带的 updatedAt(客户端时钟)新者胜。
 * 未登录 / 离线 / 服务端出错一律静默跳过 —— localStorage 始终是可用的本地缓存。 */

function pushConversation(rec: ConversationRecord): void {
  if (!kobeUser()) return;
  void commands.agentHistoryPut(rec.id, JSON.stringify(rec));
}

let syncing = false;

/** 拉取云端会话列表并与本地双向合并(登录时自动触发;可重入保护)。 */
export async function syncConversations(): Promise<void> {
  if (syncing || !kobeUser()) return;
  syncing = true;
  try {
    const res = await commands.agentHistoryList();
    if (res.status !== "ok") return;
    let list = useChatStore.getState().conversations.slice();
    const byId = new Map(list.map((c) => [c.id, c] as const));
    // 拉:云端较新或本地没有的,取全量记录
    for (const head of res.data) {
      const local = byId.get(head.id);
      if (local && local.updatedAt >= head.updated_at_ms) continue;
      const full = await commands.agentHistoryGet(head.id);
      if (full.status !== "ok") continue;
      try {
        const rec = JSON.parse(full.data) as ConversationRecord;
        if (!rec || !Array.isArray(rec.messages)) continue;
        list = local ? list.map((c) => (c.id === rec.id ? rec : c)) : [...list, rec];
      } catch {
        /* 损坏记录:跳过,不影响其余同步 */
      }
    }
    // 推:本地较新或云端没有的
    const remote = new Map(res.data.map((h) => [h.id, h.updated_at_ms] as const));
    for (const c of list) {
      const ts = remote.get(c.id);
      if (ts == null || c.updatedAt > ts) void commands.agentHistoryPut(c.id, JSON.stringify(c));
    }
    list.sort((a, b) => b.updatedAt - a.updatedAt);
    persistConversations(list);
    useChatStore.setState({ conversations: list });
  } finally {
    syncing = false;
  }
}

// 登录态变化即同步:启动时已登录(会话恢复)或用户中途登录都会触发一次。
useAppStore.subscribe(
  (s) => s.kobeUser?.id ?? null,
  (id) => {
    if (id) void syncConversations();
  },
  { fireImmediately: true },
);

/** 载入一条历史对话；仅切换 UI 投影，不影响任何在飞运行。 */
export function loadConversation(id: string): void {
  if (id === currentConvId) return;
  saveConversation(currentConvId);
  const rec = useChatStore.getState().conversations.find((c) => c.id === id);
  if (!rec) return;
  try {
    coordinator.getConversation(id);
  } catch {
    coordinator.openConversation(id, {
      messages: rec.messages,
      toolContext: contextWithRoot(rec.toolContext ?? null),
    });
  }
  currentConvId = id;
  coordinator.selectConversation(id);
}

/** 中断当前流式轮(用户按停止 / Esc)。已保留到目前为止流式出的部分助手消息。 */
export function stopTurn(): void {
  coordinator.cancelConversation(currentConvId);
}

/**
 * 发送一条用户消息。空文本忽略;正在流式则入队(本轮结束后按序自动发出),否则立即跑一轮。
 */
export async function sendMessage(raw: string): Promise<void> {
  const text = raw.trim();
  if (!text) return;
  const conversationId = currentConvId;
  await coordinator.sendMessage(conversationId, text);
  saveConversation(conversationId);
}

/** 把一条消息压入待发队列(流式期间的发送落点)。空白忽略。 */
export function enqueueMessage(raw: string): void {
  coordinator.enqueueMessage(currentConvId, raw);
}

/** 取消一条排队中的消息(用户点 × 撤回)。 */
export function dequeueQueued(index: number): void {
  coordinator.dequeueMessage(currentConvId, index);
}

/**
 * 完成一次 client-side 工具调用(ask_user_question / show_modpack 通用):把该工具 part
 * 置为 output-available(带结构化结果),可选追加一条用户回显气泡,再 run 一次 ——
 * convertToModelMessages 会据此把结果作为 tool result 喂回模型,续跑同一会话。流式中忽略。
 */
export function resolveClientTool(
  conversationId: string,
  msgId: string,
  toolCallId: string,
  output: unknown,
  echoText?: string,
): void {
  if (
    coordinator.resolveInteractiveToolForConversation(
      conversationId,
      msgId,
      toolCallId,
      output,
    )
  ) return;

  const runtime = coordinator.getConversation(conversationId);
  if (runtime.streaming) return;
  const answered = withToolOutput(runtime.messages, msgId, toolCallId, output);
  const history = echoText
    ? [...answered, { id: nextId(), role: "user", parts: [{ type: "text", text: echoText }] } as UIMessage]
    : answered;
  void coordinator.continueConversation(conversationId, history).then(() => {
    saveConversation(conversationId);
  });
}

/** 把某条消息里指定工具 part 置为 output-available(带结果)。 */
function withToolOutput(
  messages: UIMessage[],
  msgId: string,
  toolCallId: string,
  output: unknown,
): UIMessage[] {
  return messages.map((m) => {
    if (m.id !== msgId) return m;
    return {
      ...m,
      parts: m.parts.map((p) =>
        isToolPart(p) && p.toolCallId === toolCallId
          ? { ...p, state: "output-available", output }
          : p,
      ),
    } as UIMessage;
  });
}

/** 提交一次 ask_user 选择:结果 = 所选项,回显一条用户气泡。空选择忽略。 */
export function submitAskUserAnswer(
  conversationId: string,
  msgId: string,
  toolCallId: string,
  selected: string[],
): void {
  if (selected.length === 0) return;
  resolveClientTool(conversationId, msgId, toolCallId, { selected }, selected.join("、"));
}

/** 是否为工具 part(UIMessage 里工具 part 的 type 形如 "tool-<name>",带 toolCallId)。 */
function isToolPart(p: UIMessage["parts"][number]): p is Extract<
  UIMessage["parts"][number],
  { toolCallId: string }
> {
  return typeof (p as { toolCallId?: unknown }).toolCallId === "string";
}

/** 新对话:归档当前投影并切换；原对话若在运行则继续留在后台。 */
export function newChat(): void {
  saveConversation(currentConvId);
  currentConvId = mintConvId();
  coordinator.openConversation(currentConvId, {
    messages: [],
    toolContext: contextWithRoot(null),
  });
  coordinator.selectConversation(currentConvId);
}

/**
 * 从其它页面(发现 / 新建实例)带一句上下文提示打开助手:预填输入框草稿并切到助手页。
 * 不自动发送——ChatPage 取草稿后填进输入框、聚焦,由用户审阅 / 编辑再发。
 */
export function openAgentChat(prompt: string, toolContext: AgentToolContext | null = null): void {
  const current = coordinator.getConversation(currentConvId);
  const nextContext = contextWithRoot(toolContext);
  if (
    current.messages.length > 0 &&
    !sameAgentToolContext(current.toolContext as AgentToolContext, nextContext)
  ) {
    saveConversation(currentConvId);
    currentConvId = mintConvId();
    coordinator.openConversation(currentConvId, { messages: [], toolContext: nextContext });
    coordinator.selectConversation(currentConvId);
  } else {
    coordinator.setToolContext(currentConvId, nextContext);
  }
  useChatStore.setState({ draft: prompt });
  setCurrentPage("agent");
}

function sameAgentToolContext(a: AgentToolContext | null, b: AgentToolContext | null): boolean {
  return JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
}

/** ChatPage 取用一次性草稿后清空(避免重渲染 / 重挂载再次注入)。 */
export function clearDraft(): void {
  useChatStore.setState({ draft: null });
}

// ——— 上下文提示词 ———
// 由页面上下文(搜索词 + 选中的 MC 版本 / 加载器)拼一句自然语言诉求。走 t() 故跟随界面语言;
// 版本 / 加载器 / 搜索词任一为空都优雅省略,读起来仍通顺。

/** 版本 / 加载器约束子句(都为空 → 空串)。 */
function constraintClause(version: string | null, loader: string | null): string {
  const specs = [
    version ? t("agent.promptVersion", { version }) : "",
    loader ? t("agent.promptLoader", { loader }) : "",
  ].filter(Boolean);
  return specs.length ? t("agent.promptConstraints", { specs: specs.join(t("agent.promptJoin")) }) : "";
}

/** 发现页入口:搜索词(可空)+ 选中的版本 / 加载器 facet(可空)。 */
export function discoverPrompt(query: string, version: string | null, loader: string | null): string {
  const constraints = constraintClause(version, loader);
  const q = query.trim();
  return q
    ? t("agent.discoverPrompt", { query: q, constraints })
    : t("agent.discoverPromptOpen", { constraints });
}

/** 新建实例入口:当前选中的 MC 版本 / 加载器(未选则省略)。 */
export function instancePrompt(version: string | null, loader: string | null): string {
  return t("agent.instancePrompt", { constraints: constraintClause(version, loader) });
}
