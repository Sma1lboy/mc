import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Button, EmptyState, Heading, Panel, Spinner } from "../components";
import { t, useLang } from "../i18n";
import {
  useChatStore,
  sendMessage,
  newChat,
  setBrain,
  type Brain,
  type ChatMessage,
  type ToolCallPart,
} from "./chatStore";
import "./chat.css";

/**
 * ChatPage —— 整合包助手的流式对话页(方块工坊皮肤,React)。
 *
 * 布局:标题栏(标题 + 新对话) / 可滚动消息列表 / 底部输入条。
 *   - 用户气泡:右对齐、纯文本(pre-wrap)。
 *   - 助手气泡:左对齐,文本经 Streamdown 渲染(块级稳定的流式解析 + 未闭合 markdown 加固
 *     由 Streamdown 原生处理);工具调用/结果以芯片 part 按到达顺序与文本交错。
 *   - 输入:Enter 发送 / Shift+Enter 换行(输入法组合中不误触);流式期间禁用并显示指示器;
 *     内容增长时(若用户停在底部)自动滚到底。
 * 状态全部来自 chatStore(zustand 单一真相),切走本页不丢 transcript。
 *
 * Streamdown 携带 mermaid(重),仅本页按需 lazy import,其 chunk 不进主包(见 MIGRATION §0)。
 * 代码块复制/下载按钮用 Streamdown 内置 controls(样式见 chat.css),故不再需要旧的
 * 手写复制按钮 + 事件委托;分块/尾部加固也交给 Streamdown,markdownBlocks.ts 不再被消费。
 */
const Streamdown = lazy(() => import("streamdown").then((m) => ({ default: m.Streamdown })));

// 流式文本 part:整段交给 Streamdown(块级记忆,未变的块不重解析);live 时尾部显示光标。
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

// 工具调用芯片:🔧 名称 + 可展开的 JSON 参数(有参数才可展开)。
function ToolCallChip({ part }: { part: ToolCallPart }) {
  const [open, setOpen] = useState(false);
  const a = part.args;
  const hasArgs =
    a != null &&
    a !== "Null" &&
    (typeof a !== "object" || Array.isArray(a) || Object.keys(a).length > 0);
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
        <svg
          width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
          className="text-accent shrink-0" aria-hidden="true"
        >
          <path d="M14.7 6.3a4 4 0 0 0-5.2 5.2L3 18v3h3l6.5-6.5a4 4 0 0 0 5.2-5.2l-2.6 2.6-2.4-.6-.6-2.4 2.6-2.6Z" />
        </svg>
        <span className="text-faint">{t("agent.toolCall")}</span>
        <span className="font-medium text-fg">{part.name}</span>
        {hasArgs && <span className="text-muted">{open ? "▾" : "▸"}</span>}
      </button>
      {open && hasArgs && (
        <pre className="mt-[4px] max-w-full overflow-x-auto rounded-none bg-panel-2 shadow-input px-[10px] py-[8px] text-[11.5px] leading-[1.5] text-sub font-mono">
          {JSON.stringify(part.args, null, 2)}
        </pre>
      )}
    </div>
  );
}

// 工具结果芯片:✓ 名称 — 一行结果摘要。
function ToolResultChip({ name, summary }: { name: string; summary: string }) {
  return (
    <div className="my-[3px] inline-flex items-start gap-[6px] max-w-full px-[10px] py-[5px] rounded-none bg-panel-2 shadow-sunken text-[12px] leading-[1.5]">
      <svg
        width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
        strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"
        className="text-accent shrink-0 mt-[2px]" aria-hidden="true"
      >
        <path d="m5 12.5 4.5 4.5L19 7" />
      </svg>
      <span className="font-medium text-sub shrink-0">{name}</span>
      <span className="text-muted min-w-0 break-words">{summary}</span>
    </div>
  );
}

// 单条消息渲染(用户 / 助手)。
function MessageRow({ msg, last, streaming }: { msg: ChatMessage; last: boolean; streaming: boolean }) {
  if (msg.role === "user") {
    const userText = msg.parts.map((p) => (p.kind === "text" ? p.text : "")).join("");
    return (
      <div className="flex justify-end">
        <div className="max-w-[80%] px-[13px] py-[9px] rounded-none bg-accent text-accent-text shadow-raised text-[14px] leading-[1.6] whitespace-pre-wrap break-words">
          {userText}
        </div>
      </div>
    );
  }

  return (
    <div className="flex justify-start">
      <Panel variant="sunken" className="max-w-[85%] min-w-0 px-[14px] py-[11px]">
        {msg.parts.map((part, idx) => {
          switch (part.kind) {
            case "reasoning":
              return (
                <details key={idx} className="my-[4px] text-[12px]">
                  <summary className="cursor-pointer text-faint select-none">{t("agent.reasoning")}</summary>
                  <div className="mt-[4px] whitespace-pre-wrap break-words text-muted leading-[1.6] border-l-2 border-titlebar pl-[10px]">
                    {part.text}
                  </div>
                </details>
              );
            case "text":
              return (
                <AssistantText
                  key={idx}
                  text={part.text}
                  live={last && streaming && idx === msg.parts.length - 1}
                />
              );
            case "tool_call":
              return <ToolCallChip key={idx} part={part} />;
            case "tool_result":
              return <ToolResultChip key={idx} name={part.name} summary={part.summary} />;
            case "error":
              return (
                <div
                  key={idx}
                  className="my-[3px] px-[10px] py-[7px] rounded-none bg-danger-soft text-danger-text text-[12.5px] leading-[1.5] break-words"
                >
                  {part.message}
                </div>
              );
          }
        })}
        {/* 流式指示器:仅本轮最后一条助手消息在流式期间显示。 */}
        {last && streaming && (
          <div className="flex items-center gap-[7px] mt-[6px] text-[12px] text-muted">
            <Spinner size={14} />
            <span>{t("agent.streaming")}</span>
          </div>
        )}
      </Panel>
    </div>
  );
}

// 大脑开关(dev):rust|ts 分段徽标;流式期间禁用。切换即换 sendMessage 的实现路径。
function BrainToggle() {
  const brain = useChatStore((s) => s.brain);
  const streaming = useChatStore((s) => s.streaming);
  const options: Brain[] = ["rust", "ts"];
  return (
    <div className="inline-flex items-center gap-[6px] text-[12px]">
      <span className="text-faint">{t("agent.brainLabel")}</span>
      <div className="inline-flex rounded-none bg-panel-2 shadow-sunken p-[2px]">
        {options.map((b) => (
          <button
            key={b}
            type="button"
            disabled={streaming}
            onClick={() => setBrain(b)}
            className={`px-[9px] h-[22px] leading-none text-[11px] rounded-none transition-colors duration-[var(--dur)] ease-app disabled:opacity-60 ${
              brain === b ? "bg-accent text-accent-text shadow-raised" : "text-sub hover:text-fg cursor-pointer"
            }`}
          >
            {b === "rust" ? t("agent.brainRust") : t("agent.brainTs")}
          </button>
        ))}
      </div>
    </div>
  );
}

export default function ChatPage() {
  useLang();
  const messages = useChatStore((s) => s.messages);
  const streaming = useChatStore((s) => s.streaming);
  const [draft, setDraft] = useState("");
  const listEl = useRef<HTMLDivElement>(null);
  const inputEl = useRef<HTMLTextAreaElement>(null);
  // 用户是否停在底部(在底部才自动跟随滚动,滚上去看历史时不打断)。
  const pinned = useRef(true);

  const onListScroll = (): void => {
    const el = listEl.current;
    if (!el) return;
    pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  };
  // 内容变化(新消息 / 流式增量)→ 若停在底部则滚到底。
  useEffect(() => {
    const el = listEl.current;
    if (el && pinned.current) requestAnimationFrame(() => (el.scrollTop = el.scrollHeight));
  }, [messages, streaming]);

  useEffect(() => inputEl.current?.focus(), []);

  const submit = (): void => {
    const text = draft.trim();
    if (!text || streaming) return;
    setDraft("");
    pinned.current = true;
    void sendMessage(text);
    // 发送后把输入框高度收回一行。
    if (inputEl.current) inputEl.current.style.height = "auto";
  };

  const onKeyDown = (e: React.KeyboardEvent): void => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  // textarea 随内容自增高(上限 ~6 行),避免多行输入被裁。
  const autoGrow = (el: HTMLTextAreaElement): void => {
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 168)}px`;
  };

  return (
    <div className="flex flex-col h-full min-h-0">
      {/* 标题栏 */}
      <header className="shrink-0 flex items-center justify-between gap-[16px] px-[28px] py-[16px] border-b-2 border-titlebar">
        <div className="min-w-0">
          <Heading size="section" as="h1">{t("agent.title")}</Heading>
          <div className="mt-[4px] text-[12px] text-muted truncate">{t("agent.subtitle")}</div>
        </div>
        <div className="shrink-0 flex items-center gap-[12px]">
          <BrainToggle />
          <Button variant="ghost" disabled={streaming} onClick={() => void newChat()}>
            {t("agent.newChat")}
          </Button>
        </div>
      </header>

      {/* 消息列表(滚动) */}
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
          </div>
        ) : (
          <div className="h-full flex items-center justify-center">
            <EmptyState title={t("agent.emptyTitle")} hint={t("agent.emptyHint")} />
          </div>
        )}
      </div>

      {/* 输入条 */}
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
