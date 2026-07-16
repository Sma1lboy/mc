import { useEffect, useRef, useState } from "react";
import { Button, EmptyState, Heading, Panel, Select } from "../components";
import { t, useLang } from "../i18n";
import { api } from "../ipc/api";
import {
  useChatStore,
  sendMessage,
  newChat,
  clearDraft,
  dequeueQueued,
  stopTurn,
  resetAgent,
} from "./chatStore";
import { DebugTools } from "./DebugTools";
import { MessageList } from "./MessageList";
import { ShareButton } from "./ShareButton";
import { agentModeFromContext } from "./agentContext";
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

/**
 * 引擎切换(页头小下拉):OpenRouter API ↔ 本地 Claude Code(订阅,免 key)。
 * 选择持久化在 GlobalSettings.agent_provider;切换即丢弃缓存的大脑实例,下一条消息生效。
 * 本地运行时(claude + node + pnpm)未检测齐时,该选项标注「未检测到」但仍可选 ——
 * 若坚持选择,首条消息会把具体缺失报为本轮错误。
 */
function EngineSelect({ streaming }: { streaming: boolean }) {
  const [provider, setProvider] = useState<string | null>(null);
  const [runtimeOk, setRuntimeOk] = useState(true);
  useEffect(() => {
    void api.getSettings().then((s) => setProvider(s.agent_provider ?? "openrouter")).catch(() => {});
    void api
      .agentRuntimeDetect()
      .then((r) => setRuntimeOk(Boolean(r.claude_code && r.node && r.pnpm)))
      .catch(() => {});
  }, []);
  if (provider == null) return null;
  return (
    <Select
      className="shrink-0"
      value={provider}
      onChange={(v) => {
        if (streaming || v === provider) return;
        setProvider(v);
        void api
          .getSettings()
          .then((s) => api.setSettings({ ...s, agent_provider: v === "openrouter" ? null : v }))
          .catch(() => {});
        resetAgent(v);
      }}
      options={[
        { value: "openrouter", label: t("agent.engineOpenrouter") },
        {
          value: "claude-code",
          label: runtimeOk ? t("agent.engineClaudeCode") : t("agent.engineClaudeCodeMissing"),
        },
      ]}
    />
  );
}

/** 一条消息渲染(用户 / 助手)。 */
export default function ChatPage() {
  useLang();
  const messages = useChatStore((s) => s.messages);
  const streaming = useChatStore((s) => s.streaming);
  const queued = useChatStore((s) => s.queued);
  const error = useChatStore((s) => s.error);
  const toolContext = useChatStore((s) => s.toolContext);
  const pendingDraft = useChatStore((s) => s.draft);
  const isInstanceAgent = agentModeFromContext(toolContext) === "instance";
  const [draft, setDraft] = useState("");
  const listEl = useRef<HTMLDivElement>(null);
  const inputEl = useRef<HTMLTextAreaElement>(null);
  const pinned = useRef(true);
  // IME 组字守卫:WKWebView 里确认候选的那次 Enter 常带 isComposing=false,单靠它会误提交。
  const composing = useRef(false);
  const lastCompositionEnd = useRef(0);

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

  // 流式期间不再阻塞:store 决定立即发送还是入队(见 sendMessage)。
  const submit = (): void => {
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    pinned.current = true;
    void sendMessage(text);
    if (inputEl.current) inputEl.current.style.height = "auto";
  };

  const onKeyDown = (e: React.KeyboardEvent): void => {
    // 流式中按 Esc 打断当前轮(保留已流式出的部分)。
    if (e.key === "Escape" && streaming) {
      e.preventDefault();
      stopTurn();
      return;
    }
    if (e.key !== "Enter" || e.shiftKey) return;
    // 组字中 / 刚确认候选(229 或 compositionEnd 后极短窗口)一律不提交,只 Shift+Enter 换行。
    if (composing.current || e.nativeEvent.isComposing || e.keyCode === 229) return;
    if (performance.now() - lastCompositionEnd.current < 200) return;
    e.preventDefault();
    submit();
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
          <EngineSelect streaming={streaming} />
          <ShareButton />
          <Button variant="ghost" onClick={() => newChat()} className="shrink-0">
            {t("agent.newChat")}
          </Button>
        </div>
      </header>

      {isInstanceAgent && (
        <div className="shrink-0 px-[28px] py-[8px] border-b border-titlebar bg-panel-2 text-[11px] leading-[1.5] text-muted">
          {t("agent.instancePrivacyNotice")}
        </div>
      )}

      <div
        ref={listEl}
        onScroll={onListScroll}
        className="flex-1 min-h-0 overflow-y-auto px-[28px] py-[24px]"
        role="log"
        aria-live="polite"
        aria-label={t("agent.title")}
      >
        {messages.length > 0 ? (
          <div className="flex flex-col gap-[18px]">
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
        {queued.length > 0 && (
          <div className="flex flex-col items-end gap-[6px] mb-[10px]">
            {queued.map((text, i) => (
              <div
                key={i}
                className="flex items-center gap-[8px] max-w-[min(80%,600px)] px-[10px] py-[5px] border border-dashed border-titlebar text-muted text-[12.5px] leading-[1.5]"
              >
                <span className="shrink-0 text-[11px] text-faint">{t("agent.queued")}</span>
                <span className="truncate min-w-0">{text}</span>
                <button
                  type="button"
                  onClick={() => dequeueQueued(i)}
                  aria-label={t("agent.cancelQueued")}
                  className="shrink-0 text-faint hover:text-fg leading-none text-[15px]"
                >
                  ×
                </button>
              </div>
            ))}
          </div>
        )}
        <div className="flex items-end gap-[10px]">
          <Panel variant="input" className="flex-1 min-w-0 px-[12px] py-[9px]">
            <textarea
              ref={inputEl}
              value={draft}
              rows={1}
              placeholder={t("agent.placeholder")}
              onChange={(e) => {
                setDraft(e.currentTarget.value);
                autoGrow(e.currentTarget);
              }}
              onKeyDown={onKeyDown}
              onCompositionStart={() => (composing.current = true)}
              onCompositionEnd={() => {
                composing.current = false;
                lastCompositionEnd.current = performance.now();
              }}
              className="block w-full resize-none bg-transparent border-none outline-none text-fg text-[14px] leading-[1.6] placeholder:text-faint"
              style={{ maxHeight: "168px" }}
            />
          </Panel>
          {streaming ? (
            <Button variant="ghost" onClick={() => stopTurn()} className="shrink-0">
              {t("agent.stop")}
            </Button>
          ) : (
            <Button onClick={submit} disabled={!draft.trim()}>
              {t("agent.send")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
