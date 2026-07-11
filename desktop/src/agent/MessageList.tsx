import type { UIMessage } from "ai";
import { Panel, Spinner } from "../components";
import { t } from "../i18n";
import { AskUserOptions, ASK_USER_TOOL_TYPE } from "./AskUserOptions";
import {
  InstanceChangesCard,
  SHOW_INSTANCE_CHANGES_TOOL_TYPE,
} from "./InstanceChangesCard";
import { ModpackCard, SHOW_MODPACK_TOOL_TYPE } from "./ModpackCard";
import { AssistantText, ActivityGroup, isActivity, isTool } from "./ChatParts";
import { chatMessageKeys, chatPartKeys } from "./renderKeys";

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
        <div className="max-w-[min(80%,600px)] px-[13px] py-[9px] rounded-none bg-accent text-accent-text shadow-raised text-[14px] leading-[1.6] whitespace-pre-wrap break-words">
          {userText}
        </div>
      </div>
    );
  }

  // 流式指示器二选一,避免与文本光标重叠:尾段是文本时靠 AssistantText 的光标,否则底部 spinner。
  const lastPart = msg.parts[msg.parts.length - 1];
  const caretVisible = last && streaming && lastPart?.type === "text";

  // 一条消息里所有中间「思考 + 工具调用 + 进度文字」合并成 ONE 折叠块(到最后一个
  // activity 为止),只把它之后的最终回答文字 / ask_user 卡片展开显示——避免多个折叠块被
  // 进度文字打断、散成一堆。
  let lastActivity = -1;
  for (let i = 0; i < msg.parts.length; i++) {
    if (isActivity(msg.parts[i])) lastActivity = i;
  }
  const nodes: React.ReactNode[] = [];
  if (lastActivity >= 0) {
    // head:含中间进度文字,一并折进活动块。本条消息还在流式时强制展开,让工具/思考
    // 实时「流式出来」可见;turn 结束才收起(避免单个工具一完成就瞬间收起、看不到过程)。
    nodes.push(
      <ActivityGroup key="act" parts={msg.parts.slice(0, lastActivity + 1)} forceOpen={last && streaming} />,
    );
  }
  const partKeys = chatPartKeys(msg.parts);
  for (let i = lastActivity + 1; i < msg.parts.length; i++) {
    const part = msg.parts[i];
    if (part.type === "text") {
      nodes.push(
        <AssistantText key={partKeys[i]} text={part.text} live={last && streaming && i === msg.parts.length - 1} />,
      );
    } else if (isTool(part) && part.type === ASK_USER_TOOL_TYPE) {
      nodes.push(<AskUserOptions key={partKeys[i]} msgId={msg.id} part={part} globalStreaming={streaming} />);
    } else if (isTool(part) && part.type === SHOW_MODPACK_TOOL_TYPE) {
      nodes.push(<ModpackCard key={partKeys[i]} msgId={msg.id} part={part} globalStreaming={streaming} />);
    } else if (isTool(part) && part.type === SHOW_INSTANCE_CHANGES_TOOL_TYPE) {
      nodes.push(
        <InstanceChangesCard
          key={partKeys[i]}
          msgId={msg.id}
          part={part}
          globalStreaming={streaming}
        />,
      );
    }
  }

  return (
    <div className="flex justify-start">
      <Panel variant="sunken" className="max-w-[min(85%,760px)] min-w-0 px-[14px] py-[11px]">
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
  // 发送后、首个 token 到达前助手消息还没创建(最后一条仍是用户消息),此时补一个
  // 独立的「思考中…」行,免得空窗期什么都不显示。
  const pendingReply =
    streaming && (messages.length === 0 || messages[messages.length - 1].role === "user");
  const messageKeys = chatMessageKeys(messages);
  return (
    <div className="flex flex-col gap-[18px]">
      {messages.map((msg, i) => (
        <MessageRow key={messageKeys[i]} msg={msg} last={i === messages.length - 1} streaming={streaming} />
      ))}
      {pendingReply && (
        <div className="flex justify-start">
          <Panel variant="sunken" className="min-w-0 px-[14px] py-[11px]">
            <div className="flex items-center gap-[7px] text-[12px] text-muted">
              <Spinner size={14} />
              <span>{t("agent.streaming")}</span>
            </div>
          </Panel>
        </div>
      )}
    </div>
  );
}
