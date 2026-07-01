// 整合包助手聊天 store —— 模块级 Solid 信号存储(单一真相,任何组件 import 即读写)。
// ------------------------------------------------------------------
// 一条会话(sessionId)在后端保有完整 transcript:多轮对话复用同一个 sessionId,
// 每次 sendMessage 只跑「一轮」流式;newChat 调 agentChatReset 清空后端 transcript。
//
// 事件归约(reduce):把 AgentStreamEvent 折进「当前打开的助手消息」的有序 parts 里——
//   text_delta   → 追加到当前打开的文本缓冲信号(经 requestAnimationFrame 批量刷新,
//                  每帧最多重解析一次 markdown,避免逐 delta O(n²) 重解析);
//   reasoning    → 同理,追加到「思考」缓冲(纯信息,永不是答案);
//   tool_call    → 先 flush 文本、关闭当前文本 part,再压入工具调用芯片 part
//                  (下一段文本会新开一个 part,使文本与芯片按到达顺序交错);
//   tool_result  → 同 tool_call,压入工具结果芯片 part;
//   done / error → 终止本轮:flush 收尾、清 streaming;error 追加错误 part。
//
// 幂等终止:失败可能同时以 error 事件与 agentChat Promise resolve {status:"error"} 到达,
// finalize 用 terminated 标志守卫,二者视作同一次终止,不重复追加错误 part / 不卡死。
// ------------------------------------------------------------------

import { createSignal, type Accessor } from "solid-js";
import { Channel } from "@tauri-apps/api/core";
import { commands, type AgentStreamEvent, type JsonValue } from "../ipc/bindings";

export type ChatRole = "user" | "assistant";

/** 流式文本 part:内容存一个信号缓冲,rAF 刷新时整体替换(一帧一次重解析)。 */
export interface TextPart {
  kind: "text";
  text: Accessor<string>;
  /** 追加一段(内部用;rAF flush 调用)。 */
  append: (delta: string) => void;
}
/** 模型「思考」流(OpenRouter reasoning deltas):纯信息,永不是答案。 */
export interface ReasoningPart {
  kind: "reasoning";
  text: Accessor<string>;
  append: (delta: string) => void;
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

/** 一条消息:角色 + 有序 parts(parts 为信号,流式期间追加即响应式重渲染)。 */
export interface ChatMessage {
  id: string;
  role: ChatRole;
  parts: Accessor<ChatPart[]>;
  /** 追加一个 part(内部用)。 */
  pushPart: (part: ChatPart) => void;
}

// 稳定的自增 id(Date.now + 计数;仅前端展示 key,无需强随机)。
let seq = 0;
const nextId = (): string => `${Date.now().toString(36)}-${(seq++).toString(36)}`;

function makeTextPart(initial = ""): TextPart {
  const [text, setText] = createSignal(initial);
  return { kind: "text", text, append: (d) => setText((p) => p + d) };
}

function makeReasoningPart(): ReasoningPart {
  const [text, setText] = createSignal("");
  return { kind: "reasoning", text, append: (d) => setText((p) => p + d) };
}

function makeMessage(role: ChatRole, initial: ChatPart[]): ChatMessage {
  const [parts, setParts] = createSignal<ChatPart[]>(initial);
  return { id: nextId(), role, parts, pushPart: (part) => setParts((p) => [...p, part]) };
}

// ===== 会话状态(单一真相) =====

// 本次 app 运行内稳定的会话 id;多轮复用它,后端据此续接 transcript。
const sessionId = `chat-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;

const [messages, setMessages] = createSignal<ChatMessage[]>([]);
export { messages };

// 是否正在流式(禁用输入 / 显示指示器)。同一会话同一时刻只允许一轮。
const [streaming, setStreaming] = createSignal(false);
export { streaming };

// 「内容变化」滴答:流式文本经 rAF 刷入信号 / parts 追加时自增,供页面订阅以自动滚底。
// 与具体 part 信号解耦——页面只需读它一个即可响应所有流式增量。
const [chatTick, setChatTick] = createSignal(0);
export { chatTick };
const bumpTick = (): void => {
  setChatTick((t) => t + 1);
};

/**
 * 发送一条用户消息并跑一轮流式。追加「用户消息 + 一条打开的助手消息」,
 * 建一个 Channel 把每个 AgentStreamEvent 归约进助手消息的有序 parts。
 * 空文本 / 正在流式时直接忽略(一轮一次)。
 */
export async function sendMessage(raw: string): Promise<void> {
  const text = raw.trim();
  if (!text || streaming()) return;

  const asst = makeMessage("assistant", []);
  setMessages((m) => [...m, makeMessage("user", [makeTextPart(text)]), asst]);
  setStreaming(true);

  // 归约器的可变游标(仅本轮闭包内)。
  let openText: TextPart | null = null; // 当前打开的文本 part(null = 需新开)
  let openReasoning: ReasoningPart | null = null;
  let pendingText = "";
  let pendingReasoning = "";
  let rafId: number | null = null;
  let terminated = false;

  // rAF 批量刷新:把累积的 delta 一次性写进对应缓冲信号(每帧至多一次重解析)。
  const flush = (): void => {
    rafId = null;
    let changed = false;
    if (pendingText && openText) {
      openText.append(pendingText);
      pendingText = "";
      changed = true;
    }
    if (pendingReasoning && openReasoning) {
      openReasoning.append(pendingReasoning);
      pendingReasoning = "";
      changed = true;
    }
    if (changed) bumpTick();
  };
  const scheduleFlush = (): void => {
    if (rafId == null) rafId = requestAnimationFrame(flush);
  };
  const flushNow = (): void => {
    if (rafId != null) {
      cancelAnimationFrame(rafId);
      rafId = null;
    }
    flush();
  };

  const ensureText = (): TextPart => {
    if (!openText) {
      openText = makeTextPart();
      asst.pushPart(openText);
      bumpTick();
    }
    return openText;
  };
  const ensureReasoning = (): ReasoningPart => {
    if (!openReasoning) {
      openReasoning = makeReasoningPart();
      asst.pushPart(openReasoning);
      bumpTick();
    }
    return openReasoning;
  };

  // 关闭当前打开的文本/思考 part:下一段增量会新开一个,从而与芯片按顺序交错。
  const closeStreams = (): void => {
    openText = null;
    openReasoning = null;
  };

  const finalize = (errMessage?: string): void => {
    if (terminated) return;
    terminated = true;
    flushNow();
    if (errMessage) {
      asst.pushPart({ kind: "error", message: errMessage });
      bumpTick();
    }
    setStreaming(false);
  };

  const reduce = (ev: AgentStreamEvent): void => {
    switch (ev.type) {
      case "text_delta":
        ensureText();
        pendingText += ev.delta;
        scheduleFlush();
        break;
      case "reasoning":
        ensureReasoning();
        pendingReasoning += ev.delta;
        scheduleFlush();
        break;
      case "tool_call":
        flushNow();
        closeStreams();
        asst.pushPart({ kind: "tool_call", name: ev.name, args: ev.args });
        bumpTick();
        break;
      case "tool_result":
        flushNow();
        closeStreams();
        asst.pushPart({ kind: "tool_result", name: ev.name, summary: ev.summary });
        bumpTick();
        break;
      case "done":
        finalize();
        break;
      case "error":
        finalize(ev.message);
        break;
    }
  };

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

/** 新对话:清空后端 transcript 与前端消息(流式中忽略,避免半途重置)。 */
export async function newChat(): Promise<void> {
  if (streaming()) return;
  setMessages([]);
  try {
    // 清后端会话;失败不阻断(UI 已清空,下一轮仍会复用同一 sessionId)。
    await commands.agentChatReset(sessionId);
  } catch {
    /* best-effort */
  }
}
