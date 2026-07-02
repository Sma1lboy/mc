import { useState } from "react";
import { Button, toast } from "../components";
import { commands } from "../ipc/bindings";
import { t } from "../i18n";
import { useChatStore } from "./chatStore";

/* ============================================================================
 * ShareButton —— 把当前对话发布到 mc-server 换一个公开链接,复制到剪贴板。
 * 自包含:只读 chatStore 的 messages;不改任何 store 状态。空对话 / 分享中禁用。
 * 公开分享,低敏感;GET 目前返回 JSON(公开网页视图为后续)。
 * ========================================================================== */

export function ShareButton() {
  const messages = useChatStore((s) => s.messages);
  const [sharing, setSharing] = useState(false);

  const share = async (): Promise<void> => {
    if (messages.length === 0 || sharing) return;
    setSharing(true);
    try {
      const res = await commands.agentShareConversation(JSON.stringify({ messages }));
      if (res.status === "error") {
        toast({ type: "error", message: t("agent.shareFailed", { err: res.error }) });
        return;
      }
      await navigator.clipboard.writeText(res.data.url).catch(() => {});
      toast({ type: "success", message: t("agent.shareCopied") });
    } catch (e) {
      toast({ type: "error", message: t("agent.shareFailed", { err: String(e) }) });
    } finally {
      setSharing(false);
    }
  };

  return (
    <Button
      variant="ghost"
      disabled={messages.length === 0 || sharing}
      onClick={() => void share()}
      className="shrink-0"
    >
      {sharing ? t("agent.sharing") : t("agent.share")}
    </Button>
  );
}

export default ShareButton;
