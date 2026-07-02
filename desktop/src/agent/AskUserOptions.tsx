import { useState } from "react";
import clsx from "clsx";
import { t } from "../i18n";
import { submitAskUserAnswer, type AskUserPart } from "./chatStore";

/* ============================================================================
 * AskUserOptions —— 渲染模型 `ask_user_question`(原生 client-side tool)的可点选项。
 *
 * 单选/多选都是先选中(单选点选替换、多选勾选切换),另有一个常驻的自定义输入框,
 * 最后点「提交」把选中项 + 自定义文本按 toolCallId 作为该工具调用的结果喂回、续跑
 * 同一 turn(见 submitAskUserAnswer,它同时把回答作为一条用户消息插入)。
 * live=false(非末条 / 流式中 / 已答)时只读:已答的高亮 part.answer 里的选项。
 * ========================================================================== */

export function AskUserOptions(props: {
  msgId: string;
  partIdx: number;
  part: AskUserPart;
  /** 是否为当前可作答的那一条(末条助手消息、非流式、未作答)。 */
  live: boolean;
}): React.ReactElement {
  const { part, live } = props;
  const [picked, setPicked] = useState<Set<number>>(new Set());
  const [custom, setCustom] = useState("");

  const toggle = (i: number): void => {
    if (!live) return;
    setPicked((prev) => {
      if (!part.multiSelect) return prev.has(i) ? new Set<number>() : new Set([i]);
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
      .map((i) => part.options[i]?.label)
      .filter((l): l is string => !!l);
    if (customText) labels.push(customText);
    submitAskUserAnswer(props.msgId, props.partIdx, part.toolCallId, labels);
  };

  // 已答后:高亮用户选过的选项(交互态在答后失效,统一看 part.answer)。
  const answered = new Set(part.answer ?? []);

  // 流式中还没解析出选项时,先把框架/骨架渲染出来(占位行),避免空白等待。
  const skeleton = part.options.length === 0 && !part.answered;

  return (
    <div className="my-[6px] flex flex-col gap-[8px]">
      {part.question ? (
        <div className="text-[13px] text-fg leading-[1.6]">{part.question}</div>
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
        {part.options.map((opt, i) => {
          const selected = live ? picked.has(i) : answered.has(opt.label);
          return (
            <button
              key={opt.id ?? `${i}-${opt.label}`}
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
