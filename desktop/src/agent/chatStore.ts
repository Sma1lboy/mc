// 整合包助手聊天 store —— zustand(单一真相,任何组件 useChatStore 即读,任何模块 action 即写)。
// ------------------------------------------------------------------
// 大脑是 TS(@kobemc/agent-core,在 webview 内跑 AI SDK 的 tool-use loop);Rust 聊天大脑已删除。
//
// 会话就是一个 `UIMessage[]`(AI SDK 原生消息 —— 单一真相:既渲染又喂模型)。发送 / 续跑都走
// `agent.run(history, onUpdate)`:onUpdate 每次给出「正在生长的助手 UIMessage」,直接替换到列表尾;
// 文本 / 思考 / 工具调用(input-streaming → available → output 状态机)全由 AI SDK 累积,不再手写归约器。
//
// ask_user_question 是原生 client-side tool(无 executor):模型调用后 turn 暂停,该工具 part 停在
// input-available(无 output)。用户点选后把它置为 output-available,再 run 一次续跑同一会话。
// ------------------------------------------------------------------

import { create } from "zustand";
import type { UIMessage } from "ai";
// Type-only import: erased at build, so the host-agnostic brain (and its `ai`
// dependency) stays out of the main bundle — the TS path is dynamic-imported below.
import type { AgentToolContext, ModpackAgent } from "@kobemc/agent-core";
import { setCurrentPage } from "../store";
import { t } from "../i18n";

interface ChatState {
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
  /** Host 注入给 deterministic tools 的当前上下文(例如当前整合包 wiki scope)。 */
  toolContext: AgentToolContext | null;
  /** 历史对话记录(dev 调试选择器用;localStorage 持久)。 */
  conversations: ConversationRecord[];
}

export const useChatStore = create<ChatState>(() => ({
  messages: [],
  streaming: false,
  queued: [],
  error: null,
  draft: null,
  toolContext: null,
  conversations: loadConversations(),
}));

// 惰性拉起 TS 大脑(动态 import desktopAdapter → 独立 chunk,`ai` 及 provider 不进主包)。
let tsAgent: Promise<ModpackAgent> | null = null;
let tsAgentContextKey: string | null = null;

function agentContextKey(context: AgentToolContext | null | undefined): string {
  return JSON.stringify(context ?? null);
}

async function getAgent(context?: AgentToolContext | null): Promise<ModpackAgent> {
  const key = agentContextKey(context);
  if (!tsAgent || tsAgentContextKey !== key) {
    tsAgentContextKey = key;
    tsAgent = import("./desktopAdapter").then((m) => m.createDesktopAgent(context ?? undefined));
  }
  try {
    return await tsAgent;
  } catch (e) {
    tsAgent = null; // 拉起失败(缺 key / import)→ 下次重试重新初始化
    tsAgentContextKey = null;
    throw e;
  }
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

/* ——— 会话记录(dev 调试:按时间选不同对话)———
 * 每轮结束把当前对话存到 localStorage;DebugTools 据此列出、切换。dev-only,不向普通用户暴露。
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

// 把当前对话 upsert 进记录列表(每轮结束调用)。空对话不存。
function saveCurrentConversation(): void {
  const messages = useChatStore.getState().messages;
  if (messages.length === 0) return;
  const toolContext = useChatStore.getState().toolContext;
  const now = Date.now();
  const list = useChatStore.getState().conversations.slice();
  const i = list.findIndex((c) => c.id === currentConvId);
  const createdAt = i >= 0 ? list[i].createdAt : now;
  const rec: ConversationRecord = {
    id: currentConvId,
    createdAt,
    updatedAt: now,
    title: firstUserText(messages).slice(0, 60),
    messages,
    toolContext,
  };
  if (i >= 0) list[i] = rec;
  else list.unshift(rec);
  persistConversations(list);
  useChatStore.setState({ conversations: list });
}

/** 载入一条历史对话(dev):恢复 messages(即上下文),可无缝续聊。流式中忽略。 */
export function loadConversation(id: string): void {
  if (useChatStore.getState().streaming) return;
  const rec = useChatStore.getState().conversations.find((c) => c.id === id);
  if (!rec) return;
  currentConvId = id;
  useChatStore.setState({ messages: rec.messages, toolContext: rec.toolContext ?? null, error: null });
}

/**
 * 跑一轮:把给定的 UIMessage[] 历史交给大脑,流式期间把「生长中的助手消息」替换到列表尾。
 * 结束落最终 messages;失败落 error。空文本 / 拉起失败均安全兜底。
 */
// 当前在飞的一轮的中断句柄(stopTurn 用);仅活动轮期间非空。
let currentAbort: AbortController | null = null;

async function drive(history: UIMessage[]): Promise<void> {
  const abort = new AbortController();
  currentAbort = abort;
  useChatStore.setState({ streaming: true, error: null });
  try {
    const agent = await getAgent(useChatStore.getState().toolContext);
    const { messages, error } = await agent.run(
      history,
      (assistant) => {
        useChatStore.setState({ messages: [...history, assistant] });
      },
      abort.signal,
    );
    useChatStore.setState({ messages, streaming: false, error: error ?? null });
  } catch (e) {
    useChatStore.setState({ streaming: false, error: String(e) });
  }
  if (currentAbort === abort) currentAbort = null;
  saveCurrentConversation(); // 每轮结束存档,供 dev 会话选择器按时间列出
  // 本轮结束(正常完成或被中断),若有排队消息则取队首各自成一轮(递归排空,FIFO)。
  // 于是「打断当前 + 发下一条」= 打字入队 → stopTurn 中断 → 队列在此自动放行下一条。
  const queued = useChatStore.getState().queued;
  if (queued.length > 0) {
    const [next, ...rest] = queued;
    useChatStore.setState({ queued: rest });
    await sendOne(next);
  }
}

/** 中断当前流式轮(用户按停止 / Esc)。已保留到目前为止流式出的部分助手消息。 */
export function stopTurn(): void {
  currentAbort?.abort();
}

/** 追加一条用户消息到会话尾并跑一轮。 */
async function sendOne(text: string): Promise<void> {
  const userMsg: UIMessage = { id: nextId(), role: "user", parts: [{ type: "text", text }] };
  const history = [...useChatStore.getState().messages, userMsg];
  useChatStore.setState({ messages: history });
  await drive(history);
}

/**
 * 发送一条用户消息。空文本忽略;正在流式则入队(本轮结束后按序自动发出),否则立即跑一轮。
 */
export async function sendMessage(raw: string): Promise<void> {
  const text = raw.trim();
  if (!text) return;
  if (useChatStore.getState().streaming) {
    enqueueMessage(text);
    return;
  }
  await sendOne(text);
}

/** 把一条消息压入待发队列(流式期间的发送落点)。空白忽略。 */
export function enqueueMessage(raw: string): void {
  const text = raw.trim();
  if (!text) return;
  useChatStore.setState((s) => ({ queued: [...s.queued, text] }));
}

/** 取消一条排队中的消息(用户点 × 撤回)。 */
export function dequeueQueued(index: number): void {
  useChatStore.setState((s) => ({ queued: s.queued.filter((_, i) => i !== index) }));
}

/**
 * 提交一次 ask_user 选择(原生 client-side tool 做法):把该工具调用 part 置为 output-available
 * (带用户选择),再 run 一次 —— convertToModelMessages 会据此把选择作为 tool 结果喂回模型,
 * 续跑同一会话。流式中 / 空选择忽略。
 */
export function submitAskUserAnswer(msgId: string, toolCallId: string, selected: string[]): void {
  if (selected.length === 0 || useChatStore.getState().streaming) return;
  // 1) 把该 tool part 置为 output-available(模型据此拿到结构化结果、UI 标记已答)。
  const answered = useChatStore.getState().messages.map((m) => {
    if (m.id !== msgId) return m;
    return {
      ...m,
      parts: m.parts.map((p) =>
        isToolPart(p) && p.toolCallId === toolCallId
          ? { ...p, state: "output-available", output: { selected } }
          : p,
      ),
    } as UIMessage;
  });
  // 2) 追加一条用户气泡回显所选(用户视角:我的回答出现在对话流里),再续跑。
  const echo: UIMessage = {
    id: nextId(),
    role: "user",
    parts: [{ type: "text", text: selected.join("、") }],
  };
  const history = [...answered, echo];
  useChatStore.setState({ messages: history });
  void drive(history);
}

/** 是否为工具 part(UIMessage 里工具 part 的 type 形如 "tool-<name>",带 toolCallId)。 */
function isToolPart(p: UIMessage["parts"][number]): p is Extract<
  UIMessage["parts"][number],
  { toolCallId: string }
> {
  return typeof (p as { toolCallId?: unknown }).toolCallId === "string";
}

/** 新对话:归档当前对话,轮换会话 id,清空消息(流式中忽略)。 */
export function newChat(): void {
  if (useChatStore.getState().streaming) return;
  saveCurrentConversation(); // 开新对话前把当前的存档,别丢
  currentConvId = mintConvId();
  useChatStore.setState({ messages: [], error: null, queued: [], toolContext: null });
}

/**
 * 从其它页面(发现 / 新建实例)带一句上下文提示打开助手:预填输入框草稿并切到助手页。
 * 不自动发送——ChatPage 取草稿后填进输入框、聚焦,由用户审阅 / 编辑再发。
 */
export function openAgentChat(prompt: string, context?: AgentToolContext): void {
  useChatStore.setState({ draft: prompt, toolContext: context ?? null });
  setCurrentPage("agent");
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

/** 已安装整合包 Wiki 入口:明确要求先查 wiki,不让模型先做意图识别。 */
export function modpackWikiPrompt(name: string): string {
  const modpack = name.trim() || t("agent.currentModpack");
  return t("agent.modpackWikiPrompt", { name: modpack });
}
