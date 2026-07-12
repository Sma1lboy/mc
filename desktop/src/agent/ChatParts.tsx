import { lazy, Suspense, useState } from "react";
import type { UIMessage } from "ai";
import { Spinner } from "../components";
import { t } from "../i18n";
import { isActivityPart } from "./messagePartLayout";
import { RecipeCard } from "./RecipeCard";
import { parseRecipeCardBlocks } from "./recipeCards";
import { chatPartKeys, uniqueSiblingKeys } from "./renderKeys";
import "./chat.css";

/**
 * ChatParts —— ChatPage 的可复用渲染碎片(纯展示,不接 store)。
 *
 * 从 ChatPage 抽出的叶子组件与结构判断,行为与原实现逐字一致;抽出的目的是让
 * 它们既能被 ChatPage 组装,也能在隔离环境(Ladle 故事)里单独渲染 + 调 props。
 * 唯一有副作用的依赖是 store 的 AskUserOptions,故仍留在 ChatPage 里组装,这里只导出无副作用碎片。
 */

// Streamdown 携带 mermaid(重),仅按需 lazy import,其 chunk 不进主包。
const Streamdown = lazy(() => import("streamdown").then((m) => ({ default: m.Streamdown })));

// UIMessage 的部件类型(结构性判断,不依赖具体联合成员)。
export type Part = UIMessage["parts"][number];
export type ToolPart = Extract<Part, { toolCallId: string }>;
export const isTool = (p: Part): p is ToolPart =>
  typeof (p as { toolCallId?: unknown }).toolCallId === "string";
export const toolName = (p: ToolPart): string =>
  typeof p.type === "string" && p.type.startsWith("tool-") ? p.type.slice(5) : "tool";

/** 一段活动是否算「聚合活动」:连续的思考 + 非交互(ask_user / show_modpack 卡片)工具调用。 */
export const isActivity = (p: Part): boolean => isActivityPart(p);

/** 助手文本 part:整段交给 Streamdown;live 时尾部显示光标。 */
export function AssistantText({ text, live }: { text: string; live: boolean }) {
  const segments = parseRecipeCardBlocks(text);
  const segmentKeys = uniqueSiblingKeys(segments, (segment, i) =>
    segment.type === "recipe_card"
      ? `recipe-card:${segment.card.result?.id ?? segment.card.title ?? i}`
      : `markdown:${segment.text.slice(0, 48)}`,
  );
  return (
    <div className="text-[14px] leading-[1.7] text-fg break-words">
      <Suspense fallback={<div className="chat-md whitespace-pre-wrap">{text}</div>}>
        {segments.map((segment, i) =>
          segment.type === "recipe_card" ? (
            <RecipeCard key={segmentKeys[i]} card={segment.card} />
          ) : (
            <Streamdown key={segmentKeys[i]} className="chat-md">{segment.text}</Streamdown>
          ),
        )}
      </Suspense>
      {live && (
        <span className="text-accent animate-pulse select-none" aria-hidden="true">
          ▍
        </span>
      )}
    </div>
  );
}

/** 工具芯片(非 ask_user):按状态机显示 调用中 / ✓完成 / 出错,附可展开参数。 */
export function ToolChip({ part }: { part: ToolPart }) {
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

/**
 * 一段「活动」:连续的思考(reasoning)+ 工具调用,收拢成一个可展开的整体(Claude Code 风格)。
 * 进行中自动展开并显示 spinner;完成后默认收起——用户只需看最终答案,不必看中间思考/调用细节。
 */
export function ActivityGroup({ parts, forceOpen }: { parts: Part[]; forceOpen?: boolean }) {
  const tools = parts.filter(isTool);
  const partKeys = chatPartKeys(parts);
  const running = tools.some((p) => p.state === "input-streaming" || p.state === "input-available");
  const errored = tools.some((p) => p.state === "output-error");
  // 「步数」= 工具调用数(用户眼里的检索/解析步骤);纯思考无工具时回退到 part 数。
  const steps = tools.length || parts.length;
  return (
    <details className="my-[3px]" open={running || forceOpen}>
      <summary className="inline-flex items-center gap-[6px] cursor-pointer select-none text-[12px] text-faint hover:text-sub">
        {running ? (
          <Spinner size={12} />
        ) : (
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"
               strokeLinecap="round" strokeLinejoin="round"
               className={`shrink-0 ${errored ? "text-danger-text" : "text-accent"}`} aria-hidden="true">
            <path d="M14.7 6.3a4 4 0 0 0-5.2 5.2L3 18v3h3l6.5-6.5a4 4 0 0 0 5.2-5.2l-2.6 2.6-2.4-.6-.6-2.4 2.6-2.6Z" />
          </svg>
        )}
        <span>{running ? t("agent.streaming") : t("agent.activityDone", { n: String(steps) })}</span>
      </summary>
      <div className="mt-[6px] flex flex-col gap-[4px] border-l-2 border-titlebar pl-[10px]">
        {parts.map((p, i) =>
          isTool(p) ? (
            <ToolChip key={partKeys[i]} part={p} />
          ) : p.type === "reasoning" || p.type === "text" ? (
            // 折进来的中间进度文字 / 思考,以低调样式显示。
            <div key={partKeys[i]} className="whitespace-pre-wrap break-words text-muted text-[12px] leading-[1.6]">
              {p.text}
            </div>
          ) : null,
        )}
      </div>
    </details>
  );
}
