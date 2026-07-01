// 整合包助手聊天 store —— zustand(单一真相,任何组件 useChatStore 即读,任何模块 action 即写)。
// ------------------------------------------------------------------
// 一条会话(sessionId)在后端保有完整 transcript:多轮对话复用同一个 sessionId,
// 每次 sendMessage 只跑「一轮」流式;newChat 调 agentChatReset 清空后端 transcript。
//
// 事件归约(reduce):把 AgentStreamEvent 折进「当前打开的助手消息」的有序 parts 里——
//   text_delta   → 追加到当前打开的文本 part(纯字符串);
//   reasoning    → 同理,追加到「思考」part(纯信息,永不是答案);
//   tool_call    → 先关闭当前文本 part,再压入工具调用芯片 part
//                  (下一段文本会新开一个 part,使文本与芯片按到达顺序交错);
//   tool_result  → 同 tool_call,压入工具结果芯片 part;
//   done / error → 终止本轮:清 streaming;error 追加错误 part。
//
// 与 Solid 版的区别:part 内容改成普通 `string`(不再是逐帧 rAF 刷新的信号)。text_delta
// 每次直接 setState 追加——Streamdown 内部按块记忆、未变的块不重解析,逐 delta 更新成本已足够低,
// 故省掉了旧的 requestAnimationFrame 批量层与 chatTick 滴答(页面直接订阅 messages 自动滚底)。
//
// 幂等终止:失败可能同时以 error 事件与 agentChat Promise resolve {status:"error"} 到达,
// finalize 用 terminated 标志守卫,二者视作同一次终止,不重复追加错误 part / 不卡死。
// ------------------------------------------------------------------

import { create } from "zustand";
import { Channel } from "@tauri-apps/api/core";
import { commands, type AgentStreamEvent, type JsonValue } from "../ipc/bindings";
// Type-only imports: erased at build, so the host-agnostic brain (and its `ai`
// dependency) stays out of the main bundle — the TS path is dynamic-imported below.
import type { ChatMessage as BrainMessage } from "./core/types";
import type { ModpackAgent } from "./core/agent";

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

export type ChatPart = TextPart | ReasoningPart | ToolCallPart | ToolResultPart | ErrorPart;

/** 一条消息:角色 + 有序 parts。 */
export interface ChatMessage {
  id: string;
  role: ChatRole;
  parts: ChatPart[];
}

/** 大脑实现:rust = 后端 rig loop(Channel);ts = 前端 AI SDK loop(本模块)。 */
export type Brain = "rust" | "ts";

interface ChatState {
  messages: ChatMessage[];
  /** 是否正在流式(禁用输入 / 显示指示器)。同一会话同一时刻只允许一轮。 */
  streaming: boolean;
  /** 当前大脑(dev 开关);持久化到 localStorage。 */
  brain: Brain;
}

const BRAIN_KEY = "mc-launcher.agentBrain";
function readInitialBrain(): Brain {
  if (typeof window === "undefined") return "rust";
  try {
    return window.localStorage.getItem(BRAIN_KEY) === "ts" ? "ts" : "rust";
  } catch {
    return "rust";
  }
}

export const useChatStore = create<ChatState>(() => ({
  messages: [],
  streaming: false,
  brain: readInitialBrain(),
}));

/** 切换大脑(流式中忽略,避免半途换实现);持久化。 */
export function setBrain(next: Brain): void {
  if (useChatStore.getState().streaming) return;
  useChatStore.setState({ brain: next });
  try {
    window.localStorage.setItem(BRAIN_KEY, next);
  } catch {
    /* 加固 WebView 里 localStorage 可能不可用 */
  }
}

// TS 大脑的「客户端」transcript(ModelMessage[])。与 rust 路径分属两套会话:
// rust 的 transcript 存在后端(按 sessionId 续接),ts 的存在这里——中途切换大脑会
// 在另一侧从空上下文重新开始,这在本实验里可以接受。newChat 清空它。
let tsHistory: BrainMessage[] = [];
// 首次 ts 发送时惰性创建(动态 import,使 `ai` 及 provider 不进主包)。
let tsAgent: Promise<ModpackAgent> | null = null;

// 稳定的自增 id(Date.now + 计数;仅前端展示 key,无需强随机)。
let seq = 0;
const nextId = (): string => `${Date.now().toString(36)}-${(seq++).toString(36)}`;

// 本次 app 运行内稳定的会话 id;多轮复用它,后端据此续接 transcript。
const sessionId = `chat-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

/**
 * 发送一条用户消息并跑一轮流式。追加「用户消息 + 一条打开的助手消息」,
 * 建一个 Channel 把每个 AgentStreamEvent 归约进助手消息的有序 parts。
 * 空文本 / 正在流式时直接忽略(一轮一次)。
 */
export async function sendMessage(raw: string): Promise<void> {
  const text = raw.trim();
  if (!text || useChatStore.getState().streaming) return;

  const user: ChatMessage = { id: nextId(), role: "user", parts: [{ kind: "text", text }] };
  const asst: ChatMessage = { id: nextId(), role: "assistant", parts: [] };
  // 助手消息永远是数组末尾(一次只有一轮),归约器据此定位。
  const asstIdx = useChatStore.getState().messages.length + 1;
  useChatStore.setState((s) => ({ messages: [...s.messages, user, asst], streaming: true }));

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
      case "done":
        finalize();
        break;
      case "error":
        finalize(ev.message);
        break;
    }
  };

  // TS 大脑:同一个 reduce 归约器,但事件由前端 loop 直接回调(不走 Channel)。
  if (useChatStore.getState().brain === "ts") {
    await runTsTurn(text, reduce, finalize);
    return;
  }

  const ch = new Channel<AgentStreamEvent>();
  ch.onmessage = reduce;

  try {
    const res = await commands.agentChat(sessionId, text, ch);
    // 失败也会作为 error 事件到达;这里的 {status:"error"} 是同一次终止的安全网(幂等)。
    // 成功时 done 事件已在此前把本轮终止,finalize() 为无操作。
    if (res.status === "error") finalize(res.error);
    else finalize();
  } catch (e) {
    finalize(String(e));
  }
}

/**
 * 跑一轮 TS 大脑。惰性拉起 desktopAdapter(动态 import → 独立 chunk),把每个
 * AgentStreamEvent 直接喂给 reduce;core 的事件与 bindings 线格式一致,仅 args 由
 * `unknown` 转 `JsonValue`(同一份字节),故此处按需断言。runTurn 自身不 reject
 * 模型/工具错误(以 error 事件呈现);外层 catch 只兜住拉起阶段(config / import)的失败。
 */
async function runTsTurn(
  text: string,
  reduce: (ev: AgentStreamEvent) => void,
  finalize: (errMessage?: string) => void,
): Promise<void> {
  try {
    if (!tsAgent) {
      tsAgent = import("./desktopAdapter").then((m) => m.createDesktopAgent());
    }
    const agent = await tsAgent;
    const { history } = await agent.runTurn(tsHistory, text, (e) => reduce(e as AgentStreamEvent));
    tsHistory = history;
    finalize();
  } catch (e) {
    tsAgent = null; // 拉起失败(如缺 key)→ 下次重试重新初始化
    finalize(String(e));
  }
}

/** 新对话:清空后端 transcript 与前端消息(流式中忽略,避免半途重置)。 */
export async function newChat(): Promise<void> {
  if (useChatStore.getState().streaming) return;
  useChatStore.setState({ messages: [] });
  tsHistory = []; // 同时清 TS 大脑的客户端 transcript
  try {
    // 清后端会话;失败不阻断(UI 已清空,下一轮仍会复用同一 sessionId)。
    await commands.agentChatReset(sessionId);
  } catch {
    /* best-effort */
  }
}
