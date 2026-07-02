import { toast } from "../components";
import { t } from "../i18n";
import { useChatStore, currentChatSessionId, loadConversation } from "./chatStore";

/* ============================================================================
 * DebugTools —— agent 对话页的 dev 调试工具条(仅 import.meta.env.DEV 出现)。
 *
 * 每个工具是一枚 chip;要加新工具,只需往 buildTools() 返回的数组里再 push 一项
 * { key, label, title, run }。run 里可读 useChatStore.getState() 拿当前会话状态。
 * 复制类工具统一走 copyText(),自带 toast 反馈。
 * ========================================================================== */

interface DebugTool {
  key: string;
  /** chip 上的短标签(会话 / 记录 …)。 */
  label: string;
  /** hover 提示(通常含用途 + 具体值)。 */
  title: string;
  run: () => void | Promise<void>;
}

async function copyText(text: string, okMsg: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toast({ type: "success", message: okMsg });
  } catch {
    toast({ type: "error", message: t("agent.copyFailed") });
  }
}

// 当前调试工具集合。往这里加项即扩展工具条。
function buildTools(): DebugTool[] {
  const id = currentChatSessionId();
  return [
    {
      key: "session-id",
      label: t("agent.debugSessionLabel"),
      title: `${t("agent.copySessionId")}\n${id}`,
      run: () => copyText(id, t("agent.sessionIdCopied")),
    },
    {
      key: "transcript",
      label: t("agent.debugTranscriptLabel"),
      title: t("agent.copyTranscript"),
      run: () =>
        copyText(
          JSON.stringify(useChatStore.getState().messages, null, 2),
          t("agent.transcriptCopied"),
        ),
    },
  ];
}

const CopyGlyph = (): React.ReactElement => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
       strokeLinecap="round" strokeLinejoin="round" className="w-[12px] h-[12px] shrink-0" aria-hidden="true">
    <rect x="9" y="9" width="11" height="11" rx="1" />
    <path d="M5 15H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h10a1 1 0 0 1 1 1v1" />
  </svg>
);

// 会话选择器(dev):按更新时间倒序列出历史对话,选中即载入当前视图。流式中禁用。
function ConversationPicker(): React.ReactElement | null {
  const conversations = useChatStore((s) => s.conversations);
  const streaming = useChatStore((s) => s.streaming);
  if (conversations.length === 0) return null;
  const sorted = [...conversations].sort((a, b) => b.updatedAt - a.updatedAt);
  const current = currentChatSessionId();
  const label = (updatedAt: number): string =>
    new Date(updatedAt).toLocaleString(undefined, {
      month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit",
    });
  return (
    <select
      value={current}
      disabled={streaming}
      onChange={(e) => loadConversation(e.currentTarget.value)}
      title={t("agent.debugConversations")}
      className="h-[22px] max-w-[180px] px-[6px] rounded-none bg-panel-2 shadow-sunken text-[11px] text-sub cursor-pointer disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
    >
      {/* 当前会话若尚未存档(还没发消息),给个占位项,避免受控 select 值失配。 */}
      {!sorted.some((c) => c.id === current) && (
        <option value={current}>{t("agent.debugCurrentConversation")}</option>
      )}
      {sorted.map((c) => (
        <option key={c.id} value={c.id}>
          {`${label(c.updatedAt)} · ${c.title || t("agent.debugUntitledConversation")}`}
        </option>
      ))}
    </select>
  );
}

export function DebugTools(): React.ReactElement | null {
  if (!import.meta.env.DEV) return null;
  const tools = buildTools();
  return (
    <div className="inline-flex items-center gap-[6px] text-[11px]">
      <span className="text-faint">{t("agent.debugToolsLabel")}</span>
      <ConversationPicker />
      <div className="inline-flex items-center gap-[4px]">
        {tools.map((tool) => (
          <button
            key={tool.key}
            type="button"
            onClick={() => void tool.run()}
            title={tool.title}
            className="inline-flex items-center gap-[4px] h-[22px] px-[8px] rounded-none bg-panel-2 shadow-sunken text-sub hover:text-fg cursor-pointer transition-colors duration-[var(--dur)] ease-app"
          >
            <span>{tool.label}</span>
            <CopyGlyph />
          </button>
        ))}
      </div>
    </div>
  );
}

export default DebugTools;
