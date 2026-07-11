import type { UIMessage } from "ai";

/** 会话记录的 localStorage 持久化(纯读写,无 store 依赖)。云同步在 chatStore。 */
export interface ConversationRecord {
  id: string;
  createdAt: number;
  updatedAt: number;
  /** 首条用户消息(截断),用作列表标题。 */
  title: string;
  messages: UIMessage[];
}

export const CONV_KEY = "mc-launcher.agentConversations";
export const CONV_LIMIT = 50;

export function loadConversations(): ConversationRecord[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(CONV_KEY);
    const list = raw ? (JSON.parse(raw) as ConversationRecord[]) : [];
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

export function persistConversations(list: ConversationRecord[]): void {
  try {
    window.localStorage.setItem(CONV_KEY, JSON.stringify(list.slice(0, CONV_LIMIT)));
  } catch {
    /* WebView 里 localStorage 可能不可用 */
  }
}

/** 首条用户消息的纯文本(会话标题用)。 */
export function firstUserText(messages: UIMessage[]): string {
  const first = messages.find((m) => m.role === "user");
  if (!first) return "";
  return first.parts
    .map((p) => (p.type === "text" ? p.text : ""))
    .join("")
    .trim();
}

// 会话 id:一次「对话」一个,newChat 时轮换。
const mintConvId = (): string =>
  `chat-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
let currentConvId = mintConvId();

/** 当前会话 id。dev 调试面板复制它,或按 id 回溯整轮 flow。 */
export function currentChatSessionId(): string {
  return currentConvId;
}

/** 切到某条历史会话(loadConversation 用)。 */
export function setConversationId(id: string): void {
  currentConvId = id;
}

/** 开新对话:轮换会话 id。 */
export function rotateConversationId(): void {
  currentConvId = mintConvId();
}
