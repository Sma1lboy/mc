// 整合包助手聊天 store —— zustand(单一真相,任何组件 useChatStore 即读,任何模块 action 即写)。
// ------------------------------------------------------------------
// 大脑是 TS(@kobemc/agent-core,在 webview 内跑 AI SDK 的 tool-use loop);Rust 聊天大脑已删除。
// 每次 sendMessage 只跑「一轮」流式;上下文(tsHistory)存在本模块,newChat 轮换会话并清空它。
//
// 事件归约(reduce):把 AgentStreamEvent 折进「当前打开的助手消息」的有序 parts 里——
//   text_delta   → 追加到当前打开的文本 part(纯字符串);
//   reasoning    → 同理,追加到「思考」part(纯信息,永不是答案);
//   tool_call    → 先关闭当前文本 part,再压入工具调用芯片 part
//                  (下一段文本会新开一个 part,使文本与芯片按到达顺序交错);
//   tool_result  → 同 tool_call,压入工具结果芯片 part;
//   done / error → 终止本轮:清 streaming;error 追加错误 part。
//
// text_delta 每次直接 setState 追加——Streamdown 内部按块记忆、未变的块不重解析,逐 delta 更新成本足够低。
//
// 幂等终止:finalize 用 terminated 标志守卫,error 事件与 loop 抛错视作同一次终止,不重复追加 / 不卡死。
// ------------------------------------------------------------------

import { create } from "zustand";
import { type AgentStreamEvent, type JsonValue } from "../ipc/bindings";
// Type-only imports: erased at build, so the host-agnostic brain (and its `ai`
// dependency) stays out of the main bundle — the TS path is dynamic-imported below.
import type { ChatMessage as BrainMessage, ModpackAgent } from "@kobemc/agent-core";
import { setCurrentPage } from "../store";
import { t } from "../i18n";

export type ChatRole = "user" | "assistant";

/** 流式文本 part:助手可见 markdown,流式期间原地追加。 */
export interface TextPart {
  kind: "text";
  text: string;
}
/** 模型「思考」流(OpenRouter reasoning deltas):纯信息,永不是答案。 */
export interface ReasoningPart {
  kind: "reasoning";
  text: string;
}
/** 工具调用芯片:模型以 JSON 参数调用了某个确定性工具。 */
export interface ToolCallPart {
  kind: "tool_call";
  name: string;
  args: JsonValue;
}
/** 工具结果芯片:某次工具执行完成,summary 为一行人类可读结果。 */
export interface ToolResultPart {
  kind: "tool_result";
  name: string;
  summary: string;
}
/** 错误 part:本轮失败原因(终止态追加)。 */
export interface ErrorPart {
  kind: "error";
  message: string;
}
/** 提问 part:模型让用户在若干选项里点选(单/多选);答完置 answered 变为只读回显。 */
export interface AskUserPart {
  kind: "ask_user";
  /** 该 client-side tool 调用的 id;提交时按它把结果喂回、续跑同一 turn。 */
  toolCallId: string;
  question: string;
  options: { label: string; id?: string; description?: string }[];
  multiSelect: boolean;
  /** 已提交:UI 据此禁用交互并高亮已选。 */
  answered?: boolean;
  /** 用户选中的选项 label(答完回显用)。 */
  answer?: string[];
}

export type ChatPart =
  | TextPart
  | ReasoningPart
  | ToolCallPart
  | ToolResultPart
  | ErrorPart
  | AskUserPart;

/** 一条消息:角色 + 有序 parts。 */
export interface ChatMessage {
  id: string;
  role: ChatRole;
  parts: ChatPart[];
}

interface ChatState {
  messages: ChatMessage[];
  /** 是否正在流式(禁用输入 / 显示指示器)。同一会话同一时刻只允许一轮。 */
  streaming: boolean;
  /**
   * 一次性输入草稿:外部入口(发现页 / 新建实例)经 openAgentChat 预填一句上下文提示,
   * ChatPage 变为活动页后取用一次(填进输入框、聚焦,不自动发送),随即清回 null。
   */
  draft: string | null;
  /** 历史对话记录(dev 调试选择器用;localStorage 持久)。见 ConversationRecord。 */
  conversations: ConversationRecord[];
}

export const useChatStore = create<ChatState>(() => ({
  messages: [],
  streaming: false,
  draft: null,
  conversations: loadConversations(),
}));

// TS 大脑的「客户端」transcript(ModelMessage[])。与 rust 路径分属两套会话:
// rust 的 transcript 存在后端(按 sessionId 续接),ts 的存在这里——中途切换大脑会
// 在另一侧从空上下文重新开始,这在本实验里可以接受。newChat 清空它。
let tsHistory: BrainMessage[] = [];
// 首次 ts 发送时惰性创建(动态 import,使 `ai` 及 provider 不进主包)。
let tsAgent: Promise<ModpackAgent> | null = null;

// 稳定的自增 id(Date.now + 计数;仅前端展示 key,无需强随机)。
let seq = 0;
const nextId = (): string => `${Date.now().toString(36)}-${(seq++).toString(36)}`;

// 会话 id:一次「对话」一个,newChat 时轮换(见下)。rust 大脑据此续接后端 transcript。
const mintConvId = (): string =>
  `chat-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
let currentConvId = mintConvId();

/** 当前会话 id。dev 调试面板复制它,或按 id 回溯整轮 flow。 */
export function currentChatSessionId(): string {
  return currentConvId;
}

/* ——— 会话记录(dev 调试:按时间选不同对话)———
 * 每完成一轮把当前对话(messages + 时间 + 标题)存到 localStorage;DebugTools 据此列出、切换。
 * dev-only(不向普通用户暴露);记录含渲染视图 + 模型上下文,切换会话可无缝续聊。 */
export interface ConversationRecord {
  id: string;
  createdAt: number;
  updatedAt: number;
  /** 首条用户消息(截断),用作列表标题。 */
  title: string;
  /** 渲染视图(给人看)。 */
  messages: ChatMessage[];
  /**
   * ts 大脑的模型上下文(ModelMessage[])。与 messages 是同一段对话的两种表示:
   * messages 给 UI 渲染,brainHistory 喂模型。载入会话时一并还原 → 续聊无缝、不丢上下文。
   * rust 大脑的上下文存后端(按 id 续接),故此字段仅 ts 路径需要。
   */
  brainHistory?: BrainMessage[];
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

// 把当前对话 upsert 进记录列表(每轮结束调用)。空对话不存。
function saveCurrentConversation(): void {
  const msgs = useChatStore.getState().messages;
  if (msgs.length === 0) return;
  const firstUser = msgs.find((m) => m.role === "user");
  const title = firstUser
    ? firstUser.parts.map((p) => (p.kind === "text" ? p.text : "")).join("").trim().slice(0, 60)
    : "";
  const now = Date.now();
  const list = useChatStore.getState().conversations.slice();
  const i = list.findIndex((c) => c.id === currentConvId);
  const createdAt = i >= 0 ? list[i].createdAt : now;
  const rec: ConversationRecord = {
    id: currentConvId,
    createdAt,
    updatedAt: now,
    title,
    messages: msgs,
    brainHistory: tsHistory.slice(), // 存模型上下文,载入时无缝续聊
  };
  if (i >= 0) list[i] = rec;
  else list.unshift(rec);
  persistConversations(list);
  useChatStore.setState({ conversations: list });
}

/**
 * 载入一条历史对话(dev)。同时还原渲染视图(messages)与 ts 大脑模型上下文(brainHistory),
 * 故可无缝续聊。rust 大脑靠 id 切到后端已存的 transcript,同样续得上。流式中忽略。
 */
export function loadConversation(id: string): void {
  if (useChatStore.getState().streaming) return;
  const rec = useChatStore.getState().conversations.find((c) => c.id === id);
  if (!rec) return;
  currentConvId = id;
  tsHistory = rec.brainHistory ? rec.brainHistory.slice() : []; // 还原模型上下文 → 无缝续聊
  useChatStore.setState({ messages: rec.messages });
}

/**
 * 发送一条用户消息并跑一轮流式。追加「用户消息 + 一条打开的助手消息」,
 * 建一个 Channel 把每个 AgentStreamEvent 归约进助手消息的有序 parts。
 * 空文本 / 正在流式时直接忽略(一轮一次)。
 */
export async function sendMessage(raw: string): Promise<void> {
  const text = raw.trim();
  if (!text || useChatStore.getState().streaming) return;
  const user: ChatMessage = { id: nextId(), role: "user", parts: [{ kind: "text", text }] };
  useChatStore.setState((s) => ({ messages: [...s.messages, user] }));
  await streamAssistant(async (reduce) => {
    const agent = await getAgent();
    // core 事件与 bindings 线格式一致(仅 tool_call.args 由 unknown 收窄为 JsonValue,同一份字节)。
    return agent.runTurn(tsHistory, text, (e) => reduce(e as AgentStreamEvent));
  });
}

/**
 * Append an empty assistant message and stream one brain pass into it, reducing
 * every event into its ordered parts. `pass` runs the brain (runTurn / resumeTurn)
 * with the reducer and returns the updated model history to keep. One-at-a-time is
 * guarded by the callers (they check `streaming` before appending).
 */
async function streamAssistant(
  pass: (reduce: (ev: AgentStreamEvent) => void) => Promise<{ history: BrainMessage[] }>,
): Promise<void> {
  const asst: ChatMessage = { id: nextId(), role: "assistant", parts: [] };
  useChatStore.setState((s) => ({ messages: [...s.messages, asst], streaming: true }));
  // 助手消息永远是数组末尾(一次只有一轮),归约器据此定位。
  const asstIdx = useChatStore.getState().messages.length - 1;

  // 归约器的可变游标(仅本轮闭包内):当前打开的文本 / 思考 part 在 asst.parts 里的下标。
  let openTextIdx: number | null = null;
  let openReasoningIdx: number | null = null;
  let terminated = false;

  // 就地改写助手消息的 parts(不可变:新数组 / 新对象,触发订阅)。
  const patchParts = (fn: (parts: ChatPart[]) => ChatPart[]): void => {
    useChatStore.setState((s) => {
      const messages = s.messages.slice();
      const asstMsg = messages[asstIdx];
      messages[asstIdx] = { ...asstMsg, parts: fn(asstMsg.parts) };
      return { messages };
    });
  };

  // 往「当前打开的」某类文本缓冲追加;不存在则新开一个 part。
  const appendStream = (kind: "text" | "reasoning", delta: string): void => {
    patchParts((parts) => {
      const idx = kind === "text" ? openTextIdx : openReasoningIdx;
      if (idx === null) {
        if (kind === "text") openTextIdx = parts.length;
        else openReasoningIdx = parts.length;
        return [...parts, { kind, text: delta }];
      }
      const next = parts.slice();
      const p = next[idx] as TextPart | ReasoningPart;
      next[idx] = { ...p, text: p.text + delta };
      return next;
    });
  };

  // 关闭当前打开的文本/思考 part:下一段增量会新开一个,从而与芯片按顺序交错。
  const closeStreams = (): void => {
    openTextIdx = null;
    openReasoningIdx = null;
  };

  const finalize = (errMessage?: string): void => {
    if (terminated) return;
    terminated = true;
    if (errMessage) patchParts((parts) => [...parts, { kind: "error", message: errMessage }]);
    useChatStore.setState({ streaming: false });
    saveCurrentConversation(); // 每轮结束存档,供 dev 会话选择器按时间列出
  };

  const reduce = (ev: AgentStreamEvent): void => {
    switch (ev.type) {
      case "text_delta":
        appendStream("text", ev.delta);
        break;
      case "reasoning":
        appendStream("reasoning", ev.delta);
        break;
      case "tool_call":
        closeStreams();
        patchParts((parts) => [...parts, { kind: "tool_call", name: ev.name, args: ev.args }]);
        break;
      case "tool_result":
        closeStreams();
        patchParts((parts) => [...parts, { kind: "tool_result", name: ev.name, summary: ev.summary }]);
        break;
      case "ask_user": {
        closeStreams();
        // 渐进渲染:同一 tool_call_id 会从「空骨架」多次 upsert 到最终态,按 id 就地替换
        // (不存在才追加),避免每次 delta 堆一个新芯片。
        const next: AskUserPart = {
          kind: "ask_user",
          toolCallId: ev.tool_call_id,
          question: ev.question,
          // 归一化 null→undefined(bindings 的 Option 字段带 null)。
          options: ev.options.map((o) => ({
            label: o.label,
            id: o.id ?? undefined,
            description: o.description ?? undefined,
          })),
          multiSelect: ev.multi_select,
        };
        patchParts((parts) => {
          const i = parts.findIndex(
            (p) => p.kind === "ask_user" && p.toolCallId === ev.tool_call_id,
          );
          if (i < 0) return [...parts, next];
          const copy = parts.slice();
          copy[i] = next;
          return copy;
        });
        break;
      }
      case "done":
        finalize();
        break;
      case "error":
        finalize(ev.message);
        break;
    }
  };

  try {
    const { history } = await pass(reduce);
    tsHistory = history;
    finalize();
  } catch (e) {
    // 拉起阶段(缺 key / import)失败 → 记一条错误并终止(brain 自身的错误走 error 事件)。
    finalize(String(e));
  }
}

/**
 * 惰性拉起 TS 大脑(动态 import desktopAdapter → 独立 chunk,`ai` 及 provider 不进主包)。
 * 拉起失败(缺 key / import)会抛,由 streamAssistant 兜成一条 error;并清缓存以便重试。
 */
async function getAgent(): Promise<ModpackAgent> {
  if (!tsAgent) tsAgent = import("./desktopAdapter").then((m) => m.createDesktopAgent());
  try {
    return await tsAgent;
  } catch (e) {
    tsAgent = null;
    throw e;
  }
}

/**
 * 提交一次 ask_user 选择(client-side tool 的原生做法):把选中项按 toolCallId 作为该工具
 * 调用的「结果」喂回,续跑同一个 turn。同时标记该 part 已答 + 回显选中项,并把回答作为
 * 一条用户消息插入(用户视角:我的回答出现在对话流里)。流式中忽略。
 */
export function submitAskUserAnswer(
  msgId: string,
  partIdx: number,
  toolCallId: string,
  selected: string[],
): void {
  if (selected.length === 0 || useChatStore.getState().streaming) return;
  const echo: ChatMessage = {
    id: nextId(),
    role: "user",
    parts: [{ kind: "text", text: selected.join(", ") }],
  };
  useChatStore.setState((s) => ({
    messages: [
      ...s.messages.map((m) => {
        if (m.id !== msgId) return m;
        const parts = m.parts.slice();
        const p = parts[partIdx];
        if (p && p.kind === "ask_user") parts[partIdx] = { ...p, answered: true, answer: selected };
        return { ...m, parts };
      }),
      echo,
    ],
  }));
  void streamAssistant(async (reduce) => {
    const agent = await getAgent();
    return agent.resumeTurn(tsHistory, toolCallId, { selected }, (e) => reduce(e as AgentStreamEvent));
  });
}

/** 新对话:归档当前对话,轮换到新会话 id,清空前端消息与后端 transcript(流式中忽略)。 */
export function newChat(): void {
  if (useChatStore.getState().streaming) return;
  saveCurrentConversation(); // 开新对话前把当前的存档,别丢
  currentConvId = mintConvId(); // 轮换:新对话是独立记录,dev 选择器可回切旧的
  useChatStore.setState({ messages: [] });
  tsHistory = []; // 清 TS 大脑的客户端 transcript(上下文)
}

/**
 * 从其它页面(发现 / 新建实例)带一句上下文提示打开助手:预填输入框草稿并切到助手页。
 * 不自动发送——ChatPage 取草稿后填进输入框、聚焦,由用户审阅 / 编辑再发。流式中亦照常
 * 切页 + 填框(输入框在本轮结束前保持禁用,发送键同样,沿用既有行为)。
 */
export function openAgentChat(prompt: string): void {
  useChatStore.setState({ draft: prompt });
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
