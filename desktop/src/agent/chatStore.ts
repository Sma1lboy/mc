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
import type { AgentMode, AgentModeInput, ModpackAgent } from "@kobemc/agent-core";
import { setCurrentPage, useAppStore } from "../store";
import { commands } from "../ipc/bindings";
import { t } from "../i18n";
import {
  INTERACTIVE_CLIENT_TOOLS,
  isAutomaticClientTool,
  runLauncherClientTool,
} from "./clientToolDispatcher";
import { conversationRepository, mergeConversationRecords } from "./conversationRepository";

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
  /** 历史对话记录(dev 调试选择器用;launcher 本地持久)。 */
  conversations: ConversationRecord[];
  /** Host-owned deterministic tool context, never supplied by the model. */
  toolContext: AgentToolContext | null;
  /**
   * 本地引擎(claude-code)模式下「正在等用户作答」的交互 client tool 名
   * (ask_user_question / show_modpack),无则 null。此时 turn 仍在流式,
   * 但对应组件必须可交互 —— 见 AskUserOptions / ModpackCard 的 live 判定。
   */
  pendingLocalTool: string | null;
}

export const useChatStore = create<ChatState>(() => ({
  messages: [],
  streaming: false,
  queued: [],
  error: null,
  draft: null,
  conversations: [],
  toolContext: null,
  pendingLocalTool: null,
}));

/* ——— 本地引擎的 client-tool 暂停通道 ———
 * localRuntimeAdapter 把 ask_user_question / show_modpack 的 tool_call 注册到这里,
 * turn 保持打开;用户在 UI 作答后 resolveClientTool 取出 resolver、把结果发回宿主,
 * 同一轮继续。单槽(同一时刻至多一个 client tool 在等)。 */
const pendingLocalResolvers = new Map<string, (output: unknown) => void>();

/** (localRuntimeAdapter 专用)登记一个等待用户作答的 client tool。 */
export function registerLocalClientTool(name: string, resolve: (output: unknown) => void): void {
  pendingLocalResolvers.set(name, resolve);
  useChatStore.setState({ pendingLocalTool: name });
}

/** (localRuntimeAdapter 专用)清空所有待答 client tool(turn 结束 / 宿主退出)。 */
export function clearLocalClientTools(): void {
  pendingLocalResolvers.clear();
  useChatStore.setState({ pendingLocalTool: null });
}

// 惰性拉起大脑(动态 import → 独立 chunk,`ai` 及 provider 不进主包)。
// 引擎按设置选择:默认 OpenRouter API(webview 内 TS 大脑);`claude-code` =
// 本机 Claude Code 订阅(Node 宿主进程,经 localRuntimeAdapter)。
let tsAgent: Promise<ModpackAgent> | null = null;
let tsAgentKey: string | null = null;

async function getAgent(): Promise<ModpackAgent> {
  const mode = agentModeFromContext(useChatStore.getState().toolContext);
  const settings = await commands.getSettings();
  const provider =
    settings.status === "ok" ? (settings.data.agent_provider ?? "openrouter") : "openrouter";
  const key = `${provider}:${mode}`;
  if (!tsAgent || tsAgentKey !== key) {
    if (tsAgentKey?.startsWith("claude-code:")) await commands.agentHostStop().catch(() => {});
    tsAgentKey = key;
    tsAgent = (async () => {
      if (provider === "claude-code") {
        const m = await import("./localRuntimeAdapter");
        return m.createLocalRuntimeAgent(mode);
      }
      const m = await import("./desktopAdapter");
      return m.createDesktopAgent(mode);
    })();
  }
  try {
    return await tsAgent;
  } catch (e) {
    tsAgent = null; // 拉起失败(缺 key / import)→ 下次重试重新初始化
    tsAgentKey = null;
    throw e;
  }
}

/** 丢弃缓存的大脑实例(设置页切换引擎后调用;下轮消息按新设置重新拉起)。
 *  顺手停掉可能在跑的本地 Node 宿主 —— 无论切到哪个引擎都安全,下轮会按需重启。 */
export function resetAgent(): void {
  tsAgent = null;
  tsAgentKey = null;
  void import("../ipc/bindings").then((m) => m.commands.agentHostStop()).catch(() => {});
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
 * 每轮结束把当前对话存到 launcher 自己的数据目录,登录 kobeMC 账号后同时同步到
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

export interface AgentWikiContext {
  root: string;
  modpackId: string;
  instanceId: string;
  sourcePaths: string[];
}

export interface AgentInstanceContext extends AgentWikiContext {
  mcVersion: string;
  loader: string;
}

export interface AgentToolContext {
  mode?: AgentModeInput;
  instance?: AgentInstanceContext;
  /** Legacy persisted wiki-only context. New instance entrypoints use `instance`. */
  wiki?: AgentWikiContext;
}

function agentModeFromContext(context: AgentToolContext | null): AgentMode {
  const mode = context?.mode;
  if (mode === "instance" || mode === "wiki") return "instance";
  if (mode === "build" || mode === "modpack") return "build";
  return context?.instance || context?.wiki ? "instance" : "build";
}

const CONV_LIMIT = 50;

async function hydrateConversations(): Promise<void> {
  const stored = await conversationRepository.hydrate();
  const current = useChatStore.getState().conversations;
  useChatStore.setState({ conversations: mergeConversationRecords(current, stored) });
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
  const state = useChatStore.getState();
  const messages = state.messages;
  if (messages.length === 0) return;
  const now = Date.now();
  const list = state.conversations.slice();
  const i = list.findIndex((c) => c.id === currentConvId);
  const createdAt = i >= 0 ? list[i].createdAt : now;
  const rec: ConversationRecord = {
    id: currentConvId,
    createdAt,
    updatedAt: now,
    title: firstUserText(messages).slice(0, 60),
    messages,
    toolContext: state.toolContext,
  };
  if (i >= 0) list[i] = rec;
  else list.unshift(rec);
  useChatStore.setState({ conversations: list.slice(0, CONV_LIMIT) });
  conversationRepository.save(rec);
}

/* ——— 云端同步 ———
 * 登录后每次存档由 launcher host 异步镜像到 mc-server；syncConversations()
 * 也在 host 内双向合并，WebKit 始终只消费 launcher-owned state。 */

let syncing = false;

/** 登录时请求 host 合并云端会话(可重入保护)。 */
export async function syncConversations(): Promise<void> {
  if (syncing) return;
  syncing = true;
  try {
    const records = await conversationRepository.sync();
    useChatStore.setState({
      conversations: mergeConversationRecords(useChatStore.getState().conversations, records),
    });
  } finally {
    syncing = false;
  }
}

// 先恢复 launcher 本地历史，再让登录态同步补齐跨设备记录。
void hydrateConversations();

// 登录态变化即同步:启动时已登录(会话恢复)或用户中途登录都会触发一次。
useAppStore.subscribe(
  (s) => s.kobeUser?.id ?? null,
  (id) => {
    if (id) void syncConversations();
  },
  { fireImmediately: true },
);

/** 载入一条历史对话(dev):恢复 messages(即上下文),可无缝续聊。流式中忽略。 */
export function loadConversation(id: string): void {
  if (useChatStore.getState().streaming) return;
  const rec = useChatStore.getState().conversations.find((c) => c.id === id);
  if (!rec) return;
  currentConvId = id;
  useChatStore.setState({ messages: rec.messages, error: null, toolContext: rec.toolContext ?? null });
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
    const agent = await getAgent();
    const { messages, error } = await agent.run(
      history,
      (assistant) => {
        useChatStore.setState({ messages: [...history, assistant] });
      },
      abort.signal,
    );
    useChatStore.setState({ messages, streaming: false, error: error ?? null });
    if (!abort.signal.aborted && !error) {
      const resolved = await resolveAutomaticClientTools(messages);
      if (resolved?.shouldResume) {
        await drive(resolved.messages);
        return;
      }
    }
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

async function resolveAutomaticClientTools(
  messages: UIMessage[],
): Promise<{ messages: UIMessage[]; shouldResume: boolean } | null> {
  const pending = pendingAutomaticToolParts(messages);
  if (pending.length === 0) return null;

  let next = messages;
  const toolContext = useChatStore.getState().toolContext;
  for (const call of pending) {
    try {
      const output = await runLauncherClientTool(call.name, call.part.input, toolContext);
      next = withToolOutput(next, call.msgId, call.part.toolCallId, output);
    } catch (e) {
      next = withToolError(next, call.msgId, call.part.toolCallId, e instanceof Error ? e.message : String(e));
    }
    useChatStore.setState({ messages: next });
  }

  return {
    messages: next,
    shouldResume: !hasPendingInteractiveTool(next),
  };
}

function pendingAutomaticToolParts(messages: UIMessage[]): Array<{
  msgId: string;
  name: string;
  part: Extract<UIMessage["parts"][number], { toolCallId: string }> & { input?: unknown };
}> {
  const msg = [...messages].reverse().find((m) => m.role === "assistant");
  if (!msg) return [];
  return msg.parts
    .filter(isToolPart)
    .filter((p) => p.state === "input-available")
    .map((part) => ({ msgId: msg.id, name: toolNameFromPart(part), part }))
    .filter((call) => isAutomaticClientTool(call.name));
}

function hasPendingInteractiveTool(messages: UIMessage[]): boolean {
  const msg = [...messages].reverse().find((m) => m.role === "assistant");
  if (!msg) return false;
  return msg.parts
    .filter(isToolPart)
    .some((p) => p.state === "input-available" && INTERACTIVE_CLIENT_TOOLS.has(toolNameFromPart(p)));
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
 * 完成一次 client-side 工具调用(ask_user_question / show_modpack 通用):把该工具 part
 * 置为 output-available(带结构化结果),可选追加一条用户回显气泡,再 run 一次 ——
 * convertToModelMessages 会据此把结果作为 tool result 喂回模型,续跑同一会话。流式中忽略。
 */
export function resolveClientTool(
  msgId: string,
  toolCallId: string,
  output: unknown,
  echoText?: string,
): void {
  // 本地引擎路径:turn 还开着,工具在等这里的结果。把 resolver 唤醒(结果经宿主回给
  // runtime,同一轮继续流式),本地只乐观翻转该 part 的状态(下一个快照会带权威状态)。
  // 不追加回显气泡 —— 结果已作为 tool result 喂给模型,后续回答在同一条助手消息里。
  const toolName = findToolPartName(msgId, toolCallId);
  const local = toolName ? pendingLocalResolvers.get(toolName) : undefined;
  if (local && toolName) {
    pendingLocalResolvers.delete(toolName);
    useChatStore.setState({
      messages: withToolOutput(useChatStore.getState().messages, msgId, toolCallId, output),
      pendingLocalTool: pendingLocalResolvers.size ? [...pendingLocalResolvers.keys()][0] : null,
    });
    local(output);
    return;
  }

  if (useChatStore.getState().streaming) return;
  const answered = withToolOutput(useChatStore.getState().messages, msgId, toolCallId, output);
  const history = echoText
    ? [...answered, { id: nextId(), role: "user", parts: [{ type: "text", text: echoText }] } as UIMessage]
    : answered;
  useChatStore.setState({ messages: history });
  void drive(history);
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

function withToolError(
  messages: UIMessage[],
  msgId: string,
  toolCallId: string,
  errorText: string,
): UIMessage[] {
  return messages.map((m) => {
    if (m.id !== msgId) return m;
    return {
      ...m,
      parts: m.parts.map((p) =>
        isToolPart(p) && p.toolCallId === toolCallId
          ? { ...p, state: "output-error", errorText }
          : p,
      ),
    } as UIMessage;
  });
}

/** 按 msgId + toolCallId 找到工具 part 的工具名("tool-xxx" → "xxx"),找不到 → null。 */
function findToolPartName(msgId: string, toolCallId: string): string | null {
  const msg = useChatStore.getState().messages.find((m) => m.id === msgId);
  if (!msg) return null;
  for (const part of msg.parts) {
    if (isToolPart(part) && part.toolCallId === toolCallId) return toolNameFromPart(part);
  }
  return null;
}

function toolNameFromPart(part: Extract<UIMessage["parts"][number], { toolCallId: string }>): string {
  return typeof part.type === "string" && part.type.startsWith("tool-")
    ? part.type.slice("tool-".length)
    : "";
}

/** 提交一次 ask_user 选择:结果 = 所选项,回显一条用户气泡。空选择忽略。 */
export function submitAskUserAnswer(msgId: string, toolCallId: string, selected: string[]): void {
  if (selected.length === 0) return;
  resolveClientTool(msgId, toolCallId, { selected }, selected.join("、"));
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
export function openAgentChat(prompt: string, toolContext: AgentToolContext | null = null): void {
  const current = useChatStore.getState();
  if (
    current.messages.length > 0 &&
    !sameAgentToolContext(current.toolContext, toolContext)
  ) {
    saveCurrentConversation();
    currentConvId = mintConvId();
    useChatStore.setState({ messages: [], error: null, queued: [] });
  }
  useChatStore.setState({ draft: prompt, toolContext });
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
