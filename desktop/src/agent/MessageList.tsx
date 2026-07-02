import type { UIMessage } from "ai";
import { Panel, Spinner } from "../components";
import { t } from "../i18n";
import { AskUserOptions, ASK_USER_TOOL_TYPE } from "./AskUserOptions";
import { AssistantText, ActivityGroup, isActivity, isTool, type Part } from "./ChatParts";

/**
 * MessageList / MessageRow —— 组装聊天消息流(store-coupled:一条 ask_user 会经
 * AskUserOptions 回写 store)。从 ChatPage 抽出,行为逐字一致;抽出的目的是让整条
 * `UIMessage[]` 对话流既能被 ChatPage 用,也能在 Ladle 故事里喂 mock 数据整段渲染。
 */

/** 一条消息渲染(用户 / 助手)。 */
export function MessageRow({
  msg,
  last,
  streaming,
}: {
  msg: UIMessage;
  last: boolean;
  streaming: boolean;
}) {
  if (msg.role === "user") {
    const userText = msg.parts.map((p) => (p.type === "text" ? p.text : "")).join("");
    return (
      <div className="flex justify-end">
        <div className="max-w-[80%] px-[13px] py-[9px] rounded-none bg-accent text-accent-text shadow-raised text-[14px] leading-[1.6] whitespace-pre-wrap break-words">
          {userText}
        </div>
      </div>
    );
  }

  // 流式指示器二选一,避免与文本光标重叠:尾段是文本时靠 AssistantText 的光标,否则底部 spinner。
  const lastPart = msg.parts[msg.parts.length - 1];
  const caretVisible = last && streaming && lastPart?.type === "text";

  // 把「连续的思考 + 工具调用」聚成一段活动(收拢);文本 / ask_user 独立渲染。
  const nodes: React.ReactNode[] = [];
  for (let i = 0; i < msg.parts.length; ) {
    const part = msg.parts[i];
    if (isActivity(part)) {
      const run: Part[] = [];
      while (i < msg.parts.length && isActivity(msg.parts[i])) {
        run.push(msg.parts[i]);
        i++;
      }
      nodes.push(<ActivityGroup key={`act-${i}`} parts={run} />);
      continue;
    }
    if (part.type === "text") {
      nodes.push(
        <AssistantText key={i} text={part.text} live={last && streaming && i === msg.parts.length - 1} />,
      );
    } else if (isTool(part) && part.type === ASK_USER_TOOL_TYPE) {
      nodes.push(<AskUserOptions key={i} msgId={msg.id} part={part} globalStreaming={streaming} />);
    }
    i++;
  }

  return (
    <div className="flex justify-start">
      <Panel variant="sunken" className="max-w-[85%] min-w-0 px-[14px] py-[11px]">
        {nodes}
        {last && streaming && !caretVisible && (
          <div className="flex items-center gap-[7px] mt-[6px] text-[12px] text-muted">
            <Spinner size={14} />
            <span>{t("agent.streaming")}</span>
          </div>
        )}
      </Panel>
    </div>
  );
}

/** 整条消息流(与 ChatPage 列表区一致的布局)。 */
export function MessageList({
  messages,
  streaming,
}: {
  messages: UIMessage[];
  streaming: boolean;
}) {
  return (
    <div className="flex flex-col gap-[18px] max-w-[820px] mx-auto">
      {messages.map((msg, i) => (
        <MessageRow key={msg.id} msg={msg} last={i === messages.length - 1} streaming={streaming} />
      ))}
    </div>
  );
}
