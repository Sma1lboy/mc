import { lazy, Suspense, useEffect, useRef, useState } from "react";
import type { UIMessage } from "ai";
import { Button, EmptyState, Heading, Panel, Spinner } from "../components";
import { t, useLang } from "../i18n";
import { useChatStore, sendMessage, newChat, clearDraft } from "./chatStore";
import { DebugTools } from "./DebugTools";
import { AskUserOptions, ASK_USER_TOOL_TYPE } from "./AskUserOptions";
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
 * Streamdown 携带 mermaid(重),仅本页 lazy import,其 chunk 不进主包。
 */
const Streamdown = lazy(() => import("streamdown").then((m) => ({ default: m.Streamdown })));

/** 助手文本 part:整段交给 Streamdown;live 时尾部显示光标。 */
function AssistantText({ text, live }: { text: string; live: boolean }) {
  return (
    <div className="text-[14px] leading-[1.7] text-fg break-words">
      <Suspense fallback={<div className="chat-md whitespace-pre-wrap">{text}</div>}>
        <Streamdown className="chat-md">{text}</Streamdown>
      </Suspense>
      {live && (
        <span className="text-accent animate-pulse select-none" aria-hidden="true">
          ▍
        </span>
      )}
    </div>
  );
}

// UIMessage 的部件类型(结构性判断,不依赖具体联合成员)。
type Part = UIMessage["parts"][number];
type ToolPart = Extract<Part, { toolCallId: string }>;
const isTool = (p: Part): p is ToolPart => typeof (p as { toolCallId?: unknown }).toolCallId === "string";
const toolName = (p: ToolPart): string =>
  typeof p.type === "string" && p.type.startsWith("tool-") ? p.type.slice(5) : "tool";

/** 工具芯片(非 ask_user):按状态机显示 调用中 / ✓完成 / 出错,附可展开参数。 */
function ToolChip({ part }: { part: ToolPart }) {
  const [open, setOpen] = useState(false);
  const name = toolName(part);
  const done = part.state === "output-available";
  const errored = part.state === "output-error";
  const streaming = part.state === "input-streaming" || part.state === "input-available";
  const hasArgs = part.input != null && (typeof part.input !== "object" || Object.keys(part.input).length > 0);
  return (
    <div className="my-[3px]">
      <button
        type="button"
        onClick={() => hasArgs && setOpen((o) => !o)}
        className={`inline-flex items-center gap-[6px] px-[10px] h-[26px] rounded-none bg-panel-2 text-sub shadow-sunken text-[12px] leading-none whitespace-nowrap transition-colors duration-[var(--dur)] ease-app ${
          hasArgs ? "cursor-pointer hover:text-fg" : "cursor-default"
        }`}
        title={hasArgs ? t("agent.toolArgs") : undefined}
      >
        {done ? (
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4"
               strokeLinecap="round" strokeLinejoin="round" className="text-accent shrink-0" aria-hidden="true">
            <path d="m5 12.5 4.5 4.5L19 7" />
          </svg>
        ) : (
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
               strokeLinecap="round" strokeLinejoin="round"
               className={`shrink-0 ${errored ? "text-danger-text" : "text-accent"} ${streaming ? "animate-pulse" : ""}`} aria-hidden="true">
            <path d="M14.7 6.3a4 4 0 0 0-5.2 5.2L3 18v3h3l6.5-6.5a4 4 0 0 0 5.2-5.2l-2.6 2.6-2.4-.6-.6-2.4 2.6-2.6Z" />
          </svg>
        )}
        <span className="text-faint">{t("agent.toolCall")}</span>
        <span className="font-medium text-fg">{name}</span>
        {hasArgs && <span className="text-muted">{open ? "▾" : "▸"}</span>}
      </button>
      {open && hasArgs && (
        <pre className="mt-[4px] max-w-full overflow-x-auto rounded-none bg-panel-2 shadow-input px-[10px] py-[8px] text-[11.5px] leading-[1.5] text-sub font-mono">
          {JSON.stringify(part.input, null, 2)}
        </pre>
      )}
      {errored && part.errorText && (
        <div className="mt-[3px] px-[10px] py-[6px] rounded-none bg-danger-soft text-danger-text text-[12px] leading-[1.5] break-words">
          {part.errorText}
        </div>
      )}
    </div>
  );
}

/** 一组连续的工具调用:完成后默认收起(用户不需要看细节),进行中自动展开。 */
function ToolGroup({ parts }: { parts: ToolPart[] }) {
  const running = parts.some((p) => p.state === "input-streaming" || p.state === "input-available");
  const errored = parts.some((p) => p.state === "output-error");
  // 同一组通常是同一个工具的多次调用;摘要显示名字 + 次数。
  const name = toolName(parts[0]);
  const label = parts.length > 1 ? `${name} ×${parts.length}` : name;
  return (
    <details className="my-[3px]" open={running}>
      <summary className="inline-flex items-center gap-[6px] cursor-pointer select-none text-[12px] text-faint hover:text-sub">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
             strokeLinecap="round" strokeLinejoin="round"
             className={`shrink-0 ${errored ? "text-danger-text" : "text-accent"} ${running ? "animate-pulse" : ""}`} aria-hidden="true">
          <path d="M14.7 6.3a4 4 0 0 0-5.2 5.2L3 18v3h3l6.5-6.5a4 4 0 0 0 5.2-5.2l-2.6 2.6-2.4-.6-.6-2.4 2.6-2.6Z" />
        </svg>
        <span>{t("agent.toolCall")}</span>
        <span className="font-medium text-sub">{label}</span>
        {!running && !errored && <span className="text-accent">✓</span>}
      </summary>
      <div className="mt-[4px] flex flex-col gap-[3px]">
        {parts.map((p) => (
          <ToolChip key={p.toolCallId} part={p} />
        ))}
      </div>
    </details>
  );
}

/** 一条消息渲染(用户 / 助手)。 */
function MessageRow({ msg, last, streaming }: { msg: UIMessage; last: boolean; streaming: boolean }) {
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

  // 渲染前先把「连续的非 ask_user 工具调用」聚成一组(完成后收起);其余 part 逐个渲染。
  const nodes: React.ReactNode[] = [];
  for (let i = 0; i < msg.parts.length; ) {
    const part = msg.parts[i];
    if (isTool(part) && part.type !== ASK_USER_TOOL_TYPE) {
      const run: ToolPart[] = [];
      while (i < msg.parts.length) {
        const p = msg.parts[i];
        if (isTool(p) && p.type !== ASK_USER_TOOL_TYPE) {
          run.push(p);
          i++;
        } else break;
      }
      nodes.push(<ToolGroup key={`tg-${run[0].toolCallId}`} parts={run} />);
      continue;
    }
    if (part.type === "reasoning") {
      nodes.push(
        <details key={i} className="my-[4px] text-[12px]">
          <summary className="cursor-pointer text-faint select-none">{t("agent.reasoning")}</summary>
          <div className="mt-[4px] whitespace-pre-wrap break-words text-muted leading-[1.6] border-l-2 border-titlebar pl-[10px]">
            {part.text}
          </div>
        </details>,
      );
    } else if (part.type === "text") {
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
            {messages.map((msg, i) => (
              <MessageRow key={msg.id} msg={msg} last={i === messages.length - 1} streaming={streaming} />
            ))}
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
