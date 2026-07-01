import { Component, For, Index, Show, createMemo, createSignal, createEffect, onMount } from "solid-js";
import { Button, EmptyState, Heading, Panel, Spinner, toast } from "../components";
import { renderMarkdown } from "../util/markdown";
import { t } from "../i18n";
import { splitBlocks, hardenStreamingTail } from "./markdownBlocks";
import {
  messages,
  streaming,
  sendMessage,
  newChat,
  chatTick,
  type ChatMessage,
  type TextPart,
  type ToolCallPart,
} from "./chatStore";
// 复用整合包详情页沉淀下来的 `.md` markdown 样式(该文件如今只剩 `.md` 规则,
// 是全局共享的 markdown 皮肤);显式 import 确保本页无论路由图如何变动都能拿到样式。
import "../pages/ModpackDetail.css";

/**
 * ChatPage —— 整合包助手的流式对话页(方块工坊皮肤)。
 *
 * 布局:标题栏(标题 + 新对话) / 可滚动消息列表 / 底部输入条。
 *   - 用户气泡:右对齐、纯文本(pre-wrap)。
 *   - 助手气泡:左对齐,文本经 renderMarkdown 注入(store 已按帧 rAF 批量刷新缓冲,
 *     故每帧至多重解析一次);工具调用/结果以芯片 part 按到达顺序与文本交错。
 *   - 输入:Enter 发送 / Shift+Enter 换行;流式期间禁用输入并显示指示器;
 *     内容增长时(若用户停在底部)自动滚到底。
 * 状态全部来自模块级 chatStore(单一真相),切走本页不丢 transcript。
 */

// 把渲染好的 HTML 里的代码块包一层,并塞入复制按钮(点击经消息列表上的事件委托处理)。
// renderMarkdown 的两个 <pre> 生成点都精确输出 `<pre class="md-code">`,且内容已转义,
// 字符串替换安全;按钮在包装层里绝对定位,不随 <pre> 横向滚动。
function withCopyButtons(html: string): string {
  if (!html.includes('<pre class="md-code">')) return html;
  const label = t("agent.copyCode").replace(/"/g, "&quot;").replace(/</g, "&lt;");
  return html
    .replaceAll(
      '<pre class="md-code">',
      `<div class="relative group/code"><button type="button" data-copy-code aria-label="${label}" title="${label}" class="absolute top-[6px] right-[6px] z-[1] px-[7px] py-[4px] text-[11px] leading-none rounded-none bg-panel-2 text-sub shadow-raised opacity-0 group-hover/code:opacity-100 focus-visible:opacity-100 transition-opacity duration-[var(--dur)] cursor-pointer hover:text-fg">${label}</button><pre class="md-code">`,
    )
    .replaceAll("</pre>", "</pre></div>");
}

// 流式文本 part:按空行(围栏外)切块,逐块 innerHTML 渲染。
// 已完成块的源串在流式期间不变 → <Index> 的按索引信号不触发,其 DOM 完全稳定;
// 每帧只有最后一个(增长中的)块重解析。live 时对末块做渲染时尾部加固,并显示光标。
const StreamText: Component<{ part: TextPart; live: boolean }> = (props) => {
  const blocks = createMemo(() => {
    const all = splitBlocks(props.part.text());
    if (props.live && all.length) all[all.length - 1] = hardenStreamingTail(all[all.length - 1]);
    return all;
  });
  return (
    <div class="md text-[14px] leading-[1.7] text-fg break-words">
      <Index each={blocks()}>{(block) => <div innerHTML={withCopyButtons(renderMarkdown(block()))} />}</Index>
      <Show when={props.live}>
        <span class="text-accent animate-pulse select-none" aria-hidden="true">▍</span>
      </Show>
    </div>
  );
};

// 工具调用芯片:🔧 名称 + 可展开的 JSON 参数(有参数才可展开)。
const ToolCallChip: Component<{ part: ToolCallPart }> = (props) => {
  const [open, setOpen] = createSignal(false);
  const hasArgs = (): boolean => {
    const a = props.part.args;
    if (a == null || a === "Null") return false;
    if (typeof a === "object" && !Array.isArray(a)) return Object.keys(a).length > 0;
    return true;
  };
  return (
    <div class="my-[3px]">
      <button
        type="button"
        onClick={() => hasArgs() && setOpen((o) => !o)}
        class={`inline-flex items-center gap-[6px] px-[10px] h-[26px] rounded-none bg-panel-2 text-sub shadow-sunken text-[12px] leading-none whitespace-nowrap transition-colors duration-[var(--dur)] ease-app ${
          hasArgs() ? "cursor-pointer hover:text-fg" : "cursor-default"
        }`}
        title={hasArgs() ? t("agent.toolArgs") : undefined}
      >
        <svg
          width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
          stroke-width="2" stroke-linecap="round" stroke-linejoin="round"
          class="text-accent shrink-0" aria-hidden="true"
        >
          <path d="M14.7 6.3a4 4 0 0 0-5.2 5.2L3 18v3h3l6.5-6.5a4 4 0 0 0 5.2-5.2l-2.6 2.6-2.4-.6-.6-2.4 2.6-2.6Z" />
        </svg>
        <span class="text-faint">{t("agent.toolCall")}</span>
        <span class="font-medium text-fg">{props.part.name}</span>
        <Show when={hasArgs()}>
          <span class="text-muted">{open() ? "▾" : "▸"}</span>
        </Show>
      </button>
      <Show when={open() && hasArgs()}>
        <pre class="mt-[4px] max-w-full overflow-x-auto rounded-none bg-panel-2 shadow-input px-[10px] py-[8px] text-[11.5px] leading-[1.5] text-sub font-mono">
          {JSON.stringify(props.part.args, null, 2)}
        </pre>
      </Show>
    </div>
  );
};

// 工具结果芯片:✓ 名称 — 一行结果摘要。
const ToolResultChip: Component<{ name: string; summary: string }> = (props) => (
  <div class="my-[3px] inline-flex items-start gap-[6px] max-w-full px-[10px] py-[5px] rounded-none bg-panel-2 shadow-sunken text-[12px] leading-[1.5]">
    <svg
      width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"
      class="text-accent shrink-0 mt-[2px]" aria-hidden="true"
    >
      <path d="m5 12.5 4.5 4.5L19 7" />
    </svg>
    <span class="font-medium text-sub shrink-0">{props.name}</span>
    <span class="text-muted min-w-0 break-words">{props.summary}</span>
  </div>
);

// 单条消息渲染(用户 / 助手)。
const MessageRow: Component<{ msg: ChatMessage; last: boolean }> = (props) => {
  // 用户消息:合并其文本 part 为纯文本(pre-wrap 保留换行)。
  const userText = (): string =>
    props.msg
      .parts()
      .map((p) => (p.kind === "text" ? p.text() : ""))
      .join("");

  return (
    <Show
      when={props.msg.role === "assistant"}
      fallback={
        <div class="flex justify-end">
          <div class="max-w-[80%] px-[13px] py-[9px] rounded-none bg-accent text-accent-text shadow-raised text-[14px] leading-[1.6] whitespace-pre-wrap break-words">
            {userText()}
          </div>
        </div>
      }
    >
      <div class="flex justify-start">
        <Panel variant="sunken" class="max-w-[85%] min-w-0 px-[14px] py-[11px]">
          <For each={props.msg.parts()}>
            {(part, idx) => (
              <Show when={part.kind !== "reasoning"} fallback={
                <details class="my-[4px] text-[12px]">
                  <summary class="cursor-pointer text-faint select-none">{t("agent.reasoning")}</summary>
                  <div class="mt-[4px] whitespace-pre-wrap break-words text-muted leading-[1.6] border-l-2 border-titlebar pl-[10px]">
                    {part.kind === "reasoning" ? part.text() : ""}
                  </div>
                </details>
              }>
                <Show when={part.kind === "text"}>
                  {/* renderMarkdown 转义优先、仅输出白名单标签,innerHTML 安全;分块渲染见 StreamText。 */}
                  <StreamText
                    part={part as TextPart}
                    live={props.last && streaming() && idx() === props.msg.parts().length - 1}
                  />
                </Show>
                <Show when={part.kind === "tool_call"}>
                  <ToolCallChip part={part as ToolCallPart} />
                </Show>
                <Show when={part.kind === "tool_result"}>
                  <ToolResultChip
                    name={part.kind === "tool_result" ? part.name : ""}
                    summary={part.kind === "tool_result" ? part.summary : ""}
                  />
                </Show>
                <Show when={part.kind === "error"}>
                  <div class="my-[3px] px-[10px] py-[7px] rounded-none bg-danger-soft text-danger-text text-[12.5px] leading-[1.5] break-words">
                    {part.kind === "error" ? part.message : ""}
                  </div>
                </Show>
              </Show>
            )}
          </For>
          {/* 流式指示器:仅本轮最后一条助手消息在流式期间显示。 */}
          <Show when={props.last && streaming()}>
            <div class="flex items-center gap-[7px] mt-[6px] text-[12px] text-muted">
              <Spinner size={14} />
              <span>{t("agent.streaming")}</span>
            </div>
          </Show>
        </Panel>
      </div>
    </Show>
  );
};

const ChatPage: Component = () => {
  const [draft, setDraft] = createSignal("");
  let listEl: HTMLDivElement | undefined;
  let inputEl: HTMLTextAreaElement | undefined;
  // 用户是否停在底部(在底部才自动跟随滚动,滚上去看历史时不打断)。
  let pinned = true;

  const onListScroll = (): void => {
    if (!listEl) return;
    pinned = listEl.scrollHeight - listEl.scrollTop - listEl.clientHeight < 48;
  };
  // 内容变化(新消息 / 流式增量)→ 若停在底部则滚到底。
  createEffect(() => {
    messages();
    chatTick();
    if (listEl && pinned) requestAnimationFrame(() => listEl && (listEl.scrollTop = listEl.scrollHeight));
  });

  onMount(() => inputEl?.focus());

  // 复制按钮的事件委托:整个消息列表一个 click 处理器,命中 data-copy-code 时
  // 取同包装层里 <pre> 的文本写剪贴板(按钮在 pre 外,innerText 不含按钮文案)。
  const onListClick = (e: MouseEvent): void => {
    const btn = (e.target as Element).closest?.("button[data-copy-code]");
    if (!btn) return;
    const pre = btn.parentElement?.querySelector("pre");
    if (!pre) return;
    navigator.clipboard.writeText(pre.innerText).then(
      () => toast({ type: "info", message: t("agent.copied") }),
      () => toast({ type: "error", message: t("agent.copyFailed") }),
    );
  };

  const submit = (): void => {
    const text = draft().trim();
    if (!text || streaming()) return;
    setDraft("");
    pinned = true;
    void sendMessage(text);
    // 发送后把输入框高度收回一行。
    if (inputEl) inputEl.style.height = "auto";
  };

  const onKeyDown = (e: KeyboardEvent): void => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
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
    <div class="flex flex-col h-full min-h-0">
      {/* 标题栏 */}
      <header class="shrink-0 flex items-center justify-between gap-[16px] px-[28px] py-[16px] border-b-2 border-titlebar">
        <div class="min-w-0">
          <Heading size="section" as="h1">{t("agent.title")}</Heading>
          <div class="mt-[4px] text-[12px] text-muted truncate">{t("agent.subtitle")}</div>
        </div>
        <Button variant="ghost" disabled={streaming()} onClick={() => void newChat()}>
          {t("agent.newChat")}
        </Button>
      </header>

      {/* 消息列表(滚动) */}
      <div
        ref={listEl}
        onScroll={onListScroll}
        onClick={onListClick}
        class="flex-1 min-h-0 overflow-y-auto px-[28px] py-[24px]"
        role="log"
        aria-live="polite"
        aria-label={t("agent.title")}
      >
        <Show
          when={messages().length > 0}
          fallback={
            <div class="h-full flex items-center justify-center">
              <EmptyState title={t("agent.emptyTitle")} hint={t("agent.emptyHint")} />
            </div>
          }
        >
          <div class="flex flex-col gap-[18px] max-w-[820px] mx-auto">
            <For each={messages()}>
              {(msg, i) => <MessageRow msg={msg} last={i() === messages().length - 1} />}
            </For>
          </div>
        </Show>
      </div>

      {/* 输入条 */}
      <div class="shrink-0 border-t-2 border-titlebar px-[28px] py-[16px]">
        <div class="max-w-[820px] mx-auto flex items-end gap-[10px]">
          <Panel variant="input" class="flex-1 min-w-0 px-[12px] py-[9px]">
            <textarea
              ref={inputEl}
              value={draft()}
              rows={1}
              placeholder={t("agent.placeholder")}
              disabled={streaming()}
              onInput={(e) => {
                setDraft(e.currentTarget.value);
                autoGrow(e.currentTarget);
              }}
              onKeyDown={onKeyDown}
              class="block w-full resize-none bg-transparent border-none outline-none text-fg text-[14px] leading-[1.6] placeholder:text-faint disabled:opacity-60"
              style={{ "max-height": "168px" }}
            />
          </Panel>
          <Button onClick={submit} disabled={streaming() || !draft().trim()}>
            <Show when={streaming()} fallback={t("agent.send")}>
              <Spinner size={16} />
            </Show>
          </Button>
        </div>
      </div>
    </div>
  );
};

export default ChatPage;
