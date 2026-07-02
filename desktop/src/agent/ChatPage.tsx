import { useEffect, useRef, useState } from "react";
import { Button, EmptyState, Heading, Panel, Spinner } from "../components";
import { t, useLang } from "../i18n";
import { useChatStore, sendMessage, newChat, clearDraft } from "./chatStore";
import { DebugTools } from "./DebugTools";
import { MessageList } from "./MessageList";
import "./chat.css";

/**
 * ChatPage —— 整合包助手的流式对话页(方块工坊皮肤,React)。
 *
 * 直接渲染 AI SDK 原生 `UIMessage.parts`(text / reasoning / tool),不再走自定义事件 + 手写归约:
 *   - 用户气泡:右对齐、纯文本(pre-wrap)。
 *   - 助手气泡:左对齐,文本经 Streamdown 渲染;工具 part 按状态机(input-streaming → available →
 *     output)渲染;ask_user_question 工具渲染为可点选项(AskUserOptions)。
 *   - 输入:Enter 发送 / Shift+Enter 换行;流式期间禁用并显示指示器;停在底部时自动跟随滚动。
 *
 * 无副作用的渲染碎片(文本 / 工具芯片 / 活动块)抽到 ChatParts.tsx,便于隔离预览(Ladle)。
 */

/** 一条消息渲染(用户 / 助手)。 */
export default function ChatPage() {
  useLang();
  const messages = useChatStore((s) => s.messages);
  const streaming = useChatStore((s) => s.streaming);
  const error = useChatStore((s) => s.error);
  const pendingDraft = useChatStore((s) => s.draft);
  const [draft, setDraft] = useState("");
  const listEl = useRef<HTMLDivElement>(null);
  const inputEl = useRef<HTMLTextAreaElement>(null);
  const pinned = useRef(true);

  const onListScroll = (): void => {
    const el = listEl.current;
    if (!el) return;
    pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  };
  useEffect(() => {
    const el = listEl.current;
    if (el && pinned.current) requestAnimationFrame(() => (el.scrollTop = el.scrollHeight));
  }, [messages, streaming]);

  useEffect(() => inputEl.current?.focus(), []);

  useEffect(() => {
    if (pendingDraft == null) return;
    setDraft(pendingDraft);
    clearDraft();
    requestAnimationFrame(() => {
      const el = inputEl.current;
      if (!el) return;
      el.focus();
      el.style.height = "auto";
      el.style.height = `${Math.min(el.scrollHeight, 168)}px`;
    });
  }, [pendingDraft]);

  const submit = (): void => {
    const text = draft.trim();
    if (!text || streaming) return;
    setDraft("");
    pinned.current = true;
    void sendMessage(text);
    if (inputEl.current) inputEl.current.style.height = "auto";
  };

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  const autoGrow = (el: HTMLTextAreaElement): void => {
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 168)}px`;
  };

  return (
    <div className="flex flex-col h-full min-h-0">
      <header className="shrink-0 flex items-center justify-between gap-[16px] px-[28px] py-[16px] border-b-2 border-titlebar">
        <div className="min-w-0">
          <Heading size="section" as="h1">{t("agent.title")}</Heading>
          <div className="mt-[4px] text-[12px] text-muted truncate">{t("agent.subtitle")}</div>
        </div>
        <div className="flex items-center gap-[12px] min-w-0">
          <div className="min-w-0 overflow-x-auto">
            <DebugTools />
          </div>
          <Button variant="ghost" disabled={streaming} onClick={() => newChat()} className="shrink-0">
            {t("agent.newChat")}
          </Button>
        </div>
      </header>

      <div
        ref={listEl}
        onScroll={onListScroll}
        className="flex-1 min-h-0 overflow-y-auto px-[28px] py-[24px]"
        role="log"
        aria-live="polite"
        aria-label={t("agent.title")}
      >
        {messages.length > 0 ? (
          <div className="flex flex-col gap-[18px] max-w-[820px] mx-auto">
            <MessageList messages={messages} streaming={streaming} />
            {error && (
              <div className="max-w-[85%] px-[10px] py-[7px] rounded-none bg-danger-soft text-danger-text text-[12.5px] leading-[1.5] break-words">
                {error}
              </div>
            )}
          </div>
        ) : (
          <div className="h-full flex items-center justify-center">
            <EmptyState title={t("agent.emptyTitle")} hint={t("agent.emptyHint")} />
          </div>
        )}
      </div>

      <div className="shrink-0 border-t-2 border-titlebar px-[28px] py-[16px]">
        <div className="max-w-[820px] mx-auto flex items-end gap-[10px]">
          <Panel variant="input" className="flex-1 min-w-0 px-[12px] py-[9px]">
            <textarea
              ref={inputEl}
              value={draft}
              rows={1}
              placeholder={t("agent.placeholder")}
              disabled={streaming}
              onChange={(e) => {
                setDraft(e.currentTarget.value);
                autoGrow(e.currentTarget);
              }}
              onKeyDown={onKeyDown}
              className="block w-full resize-none bg-transparent border-none outline-none text-fg text-[14px] leading-[1.6] placeholder:text-faint disabled:opacity-60"
              style={{ maxHeight: "168px" }}
            />
          </Panel>
          <Button onClick={submit} disabled={streaming || !draft.trim()}>
            {streaming ? <Spinner size={16} /> : t("agent.send")}
          </Button>
        </div>
      </div>
    </div>
  );
}
