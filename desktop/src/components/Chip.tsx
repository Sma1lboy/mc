import type { ReactNode } from "react";
import { t } from "../i18n";

/* ============================================================================
 * Chip —— Blocky Craft 可点芯片(内容类型 tab / 实例子 tab / 筛选项)。
 *
 * 三态:
 *   active           选中 —— 熔岩橙凸起倒角 + 深色文字。
 *   默认(未选)       —— panel-2 底 + 次要文字 + 浅凹。
 *   onRemove 提供时   —— 「已选筛选」沙金底 + 深色文字 + 可点 ✕(可叠加 active 忽略)。
 *
 * 方块感:直角(rounded-none),立体靠倒角阴影。文案 12px,中英混排走 Noto。
 * ========================================================================== */

export interface ChipProps {
  children: ReactNode;
  /** 选中态(熔岩橙凸起)。 */
  active?: boolean;
  onClick?: (e: MouseEvent) => void;
  /** 提供则渲染可点 ✕,样式转「已选筛选」沙金芯片。 */
  onRemove?: () => void;
  /** ✕ 的无障碍标签,默认「移除」。 */
  removeLabel?: string;
  className?: string;
  title?: string;
}

const BASE =
  "inline-flex items-center gap-[6px] px-[12px] h-[30px] rounded-none " +
  "text-[12px] font-medium leading-none cursor-pointer select-none whitespace-nowrap " +
  "transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app";

export function Chip(props: ChipProps): React.ReactElement {
  const { children, active, onClick, onRemove, removeLabel, className, title } = props;

  // 「已选筛选」沙金芯片(可移除)优先;否则在 选中(accent)/未选(panel-2)间切。
  const tone = onRemove
    ? "bg-tag text-[#16170f]"
    : active
      ? "bg-accent text-accent-text shadow-raised"
      : "bg-panel-2 text-sub shadow-sunken hover:text-fg";

  return (
    <span
      title={title}
      role="button"
      tabIndex={0}
      className={`${BASE} ${tone}${className ? " " + className : ""}`}
      onClick={(e) => onClick?.(e.nativeEvent)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick?.(e.nativeEvent as unknown as MouseEvent);
        }
      }}
    >
      {children}
      {onRemove && (
        <button
          type="button"
          className="inline-flex items-center justify-center -mr-[2px] h-[16px] w-[16px] border-none bg-transparent text-[#16170f] cursor-pointer opacity-80 hover:opacity-100 focus-visible:outline-none"
          aria-label={removeLabel ?? t("components.chip.remove")}
          onClick={(e) => {
            e.stopPropagation();
            onRemove?.();
          }}
        >
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none" aria-hidden="true">
            <path d="M3 3l6 6M9 3l-6 6" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
          </svg>
        </button>
      )}
    </span>
  );
}

export default Chip;
