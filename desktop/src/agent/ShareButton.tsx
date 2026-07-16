import { useEffect, useState } from "react";
import { Button, toast } from "../components";
import { commands } from "../ipc/bindings";
import { t } from "../i18n";
import { useChatStore } from "./chatStore";
import { projectMessagesForPublicShare } from "./conversationPrivacy";

/* ============================================================================
 * ShareButton —— 把当前对话发布到 mc-server 换一个公开链接。分享成功后在头部
 * 右上角显示生成的链接(点击可再次复制);对话内容变化(旧快照失效)即清掉。
 * 自包含:只读 chatStore 的 messages;不改任何 store 状态。空对话 / 分享中禁用。
 * 公开分享,低敏感;GET 目前返回 JSON(公开网页视图为后续)。
 * ========================================================================== */

export function ShareButton() {
  const messages = useChatStore((s) => s.messages);
  const [sharing, setSharing] = useState(false);
  const [sharedUrl, setSharedUrl] = useState<string | null>(null);

  // 对话内容变了(条数变化)→ 之前分享的是旧快照,清掉链接,重新分享才反映新内容。
  useEffect(() => {
    setSharedUrl(null);
  }, [messages.length]);

  const share = async (): Promise<void> => {
    if (messages.length === 0 || sharing) return;
    setSharing(true);
    try {
      const publicMessages = projectMessagesForPublicShare(messages);
      const res = await commands.agentShareConversation(JSON.stringify({ messages: publicMessages }));
      if (res.status === "error") {
        toast({ type: "error", message: t("agent.shareFailed", { err: res.error }) });
        return;
      }
      setSharedUrl(res.data.url);
      await navigator.clipboard.writeText(res.data.url).catch(() => {});
      toast({ type: "success", message: t("agent.shareCopied") });
    } catch (e) {
      toast({ type: "error", message: t("agent.shareFailed", { err: String(e) }) });
    } finally {
      setSharing(false);
    }
  };

  const copyAgain = async (): Promise<void> => {
    if (!sharedUrl) return;
    await navigator.clipboard.writeText(sharedUrl).catch(() => {});
    toast({ type: "success", message: t("agent.shareCopied") });
  };

  return (
    <div className="flex items-center gap-[8px] min-w-0">
      {sharedUrl && (
        <button
          type="button"
          onClick={() => void copyAgain()}
          title={t("agent.shareCopyAgain")}
          className="flex items-center gap-[6px] min-w-0 max-w-[240px] px-[9px] h-[28px] rounded-none bg-panel-2 text-sub shadow-sunken text-[12px] leading-none cursor-pointer hover:text-fg transition-colors duration-[var(--dur)] ease-app"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
               strokeLinecap="round" strokeLinejoin="round" className="shrink-0 text-accent" aria-hidden="true">
            <path d="M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1" />
            <path d="M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1" />
          </svg>
          <span className="truncate">{sharedUrl}</span>
        </button>
      )}
      <Button
        variant="ghost"
        disabled={messages.length === 0 || sharing}
        onClick={() => void share()}
        className="shrink-0"
      >
        {sharing ? t("agent.sharing") : t("agent.share")}
      </Button>
    </div>
  );
}

export default ShareButton;
