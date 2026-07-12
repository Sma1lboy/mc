import { useState } from "react";
import type { UIMessage } from "ai";
import clsx from "clsx";
import { t } from "../i18n";
import { submitAskUserAnswer, useChatStore } from "./chatStore";
import { askOptionKeys } from "./renderKeys";

/* ============================================================================
 * AskUserOptions —— 渲染 `ask_user_question`(原生 client-side tool)的可点选项。
 *
 * 直接读 UIMessage 里那个 ToolUIPart 的状态机:
 *   input-streaming  → args 还在流,先渲染骨架 + 已解析出的部分(AI SDK 给 partial input)。
 *   input-available  → args 完整、turn 已暂停等用户 → 可作答(单选点选即提交;多选勾选后提交)。
 *   output-available → 已答,回显用户选过的项(高亮、只读)。
 * 提交把选择写回该 tool part 的 output,再续跑同一会话(见 submitAskUserAnswer)。
 * ========================================================================== */

/** UIMessage 里 ask_user 工具 part 的 type。 */
export const ASK_USER_TOOL_TYPE = "tool-ask_user_question";

type ToolPart = Extract<UIMessage["parts"][number], { toolCallId: string }>;

interface AskInput {
  question?: string;
  options?: { label?: string; id?: string; description?: string }[];
  multi_select?: boolean;
}

export function AskUserOptions(props: {
  msgId: string;
  part: ToolPart;
  /** 全局是否在流式(input-available 且非流式时才可作答)。 */
  globalStreaming: boolean;
}): React.ReactElement {
  const { part, globalStreaming } = props;
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [custom, setCustom] = useState("");

  const input = (part.input ?? {}) as AskInput;
  const question = typeof input.question === "string" ? input.question : "";
  const multiSelect = input.multi_select === true;
  // 宽容外部模型输入:本地 runtime 不像 AI SDK loop 那样硬校验工具入参,
  // 模型偶尔会把 options 发成裸字符串数组 —— 归一成 {label}。
  const options = (Array.isArray(input.options) ? input.options : [])
    .map((o) => (typeof o === "string" ? { label: o } : o))
    .filter(
      (o): o is { label: string; id?: string; description?: string } =>
        !!o && typeof o.label === "string",
    );

  const answered = part.state === "output-available";
  const chosen = new Set<string>(
    answered && Array.isArray((part.output as { selected?: string[] } | undefined)?.selected)
      ? (part.output as { selected: string[] }).selected
      : [],
  );
  // 本地引擎(claude-code)下 turn 暂停等答时 streaming 仍为 true —— 以待答标记放行。
  const pendingLocalToolCallIds = useChatStore((s) => s.pendingLocalToolCallIds);
  const conversationId = useChatStore((s) => s.conversationId);
  const live =
    part.state === "input-available" &&
    (!globalStreaming || pendingLocalToolCallIds.includes(part.toolCallId));
  const skeleton = options.length === 0 && !answered;
  const optionKeys = askOptionKeys(options);

  const toggle = (i: number): void => {
    if (!live) return;
    setPicked((prev) => {
      if (!multiSelect) return prev.has(i) ? new Set<number>() : new Set([i]);
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  };

  const customText = custom.trim();
  const canSubmit = live && (picked.size > 0 || customText.length > 0);

  const submit = (): void => {
    if (!canSubmit) return;
    const labels = [...picked]
      .sort((a, b) => a - b)
      .map((i) => options[i]?.label)
      .filter((l): l is string => !!l);
    if (customText) labels.push(customText);
    submitAskUserAnswer(conversationId, props.msgId, part.toolCallId, labels);
  };

  return (
    <div className="my-[6px] flex flex-col gap-[8px]">
      {question ? (
        <div className="text-[13px] text-fg leading-[1.6]">{question}</div>
      ) : (
        skeleton && <div className="h-[18px] w-[60%] rounded-none bg-panel-2 animate-pulse" aria-hidden="true" />
      )}

      {skeleton && (
        <div className="flex flex-col gap-[6px]" aria-hidden="true">
          {[0, 1, 2].map((i) => (
            <div key={i} className="h-[38px] rounded-none border border-titlebar bg-panel-2 shadow-input animate-pulse" />
          ))}
        </div>
      )}

      <div className="flex flex-col gap-[6px]">
        {options.map((opt, i) => {
          const selected = answered ? chosen.has(opt.label) : picked.has(i);
          return (
            <button
              key={optionKeys[i]}
              type="button"
              disabled={!live}
              aria-pressed={selected}
              onClick={() => toggle(i)}
              className={clsx(
                "text-left px-[11px] py-[8px] rounded-none border shadow-input transition-colors duration-[var(--dur)] ease-app",
                live ? "cursor-pointer" : "cursor-default opacity-70",
                selected
                  ? "bg-panel-3 border-accent text-strong"
                  : "bg-panel-2 border-titlebar text-fg hover:enabled:border-accent",
              )}
            >
              <div className="flex items-center gap-[8px]">
                <span
                  className={clsx(
                    "shrink-0 w-[14px] h-[14px] grid place-items-center border rounded-none",
                    selected ? "bg-accent border-accent text-accent-text" : "border-muted",
                  )}
                  aria-hidden="true"
                >
                  {selected && (
                    <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor"
                         strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                      <path d="m5 12.5 4.5 4.5L19 7" />
                    </svg>
                  )}
                </span>
                <span className="text-[13px] font-medium">{opt.label}</span>
              </div>
              {opt.description && (
                <div className="mt-[2px] text-[12px] text-muted leading-[1.5]">{opt.description}</div>
              )}
            </button>
          );
        })}
      </div>

      {live && (
        <>
          <input
            type="text"
            value={custom}
            onChange={(e) => setCustom(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit();
            }}
            placeholder={t("agent.askCustomPlaceholder")}
            className="px-[11px] py-[8px] rounded-none border border-titlebar bg-panel-2 text-fg text-[13px] shadow-input placeholder:text-faint focus:border-accent focus:outline-none transition-colors duration-[var(--dur)] ease-app"
          />
          <button
            type="button"
            disabled={!canSubmit}
            onClick={submit}
            className="self-start px-[14px] py-[7px] rounded-none bg-accent text-accent-text shadow-raised text-[13px] font-medium cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed hover:enabled:bg-accent-hover transition-colors duration-[var(--dur)] ease-app"
          >
            {t("agent.askSubmit")}
          </button>
        </>
      )}
    </div>
  );
}

export default AskUserOptions;
