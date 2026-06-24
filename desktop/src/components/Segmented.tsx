import { For, JSX } from "solid-js";

/* ============================================================================
 * Segmented —— Blocky Craft 分段控件(单选)。
 *
 * 一条深凹轨(panel-2 + shadow-input)里放若干段;选中段熔岩橙凸起 + 深色文字,
 * 未选透明 + 弱化文字。用于:主题 / 语言 / 下载源 / 源切换(Modrinth·CurseForge)/
 * 内容类型 / 实例子 tab。`pixel` 用 Press Start 2P(仅英文短词如 Modrinth)。
 * ========================================================================== */

export interface SegmentedOption<T extends string> {
  value: T;
  label: JSX.Element;
  title?: string;
}

export interface SegmentedProps<T extends string> {
  options: readonly SegmentedOption<T>[];
  value: T;
  onChange: (value: T) => void;
  /** 段文案用点阵体(Press Start 2P);仅当全是英文短词时启用。 */
  pixel?: boolean;
  /** 整条控件的无障碍标签。 */
  ariaLabel?: string;
  class?: string;
}

export function Segmented<T extends string>(props: SegmentedProps<T>): JSX.Element {
  return (
    <div
      role="radiogroup"
      aria-label={props.ariaLabel}
      class={`inline-flex p-[3px] bg-panel-2 shadow-input rounded-none${
        props.class ? " " + props.class : ""
      }`}
    >
      <For each={props.options}>
        {(opt) => {
          const selected = (): boolean => props.value === opt.value;
          return (
            <button
              type="button"
              role="radio"
              aria-checked={selected()}
              title={opt.title}
              class={`inline-flex items-center justify-center px-[14px] h-[30px] rounded-none border-none cursor-pointer select-none whitespace-nowrap leading-none transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app ${
                props.pixel ? "font-pixel text-[10px]" : "font-medium text-[12px]"
              } ${
                selected()
                  ? "bg-accent text-accent-text shadow-raised"
                  : "bg-transparent text-muted hover:text-sub"
              }`}
              onClick={() => !selected() && props.onChange(opt.value)}
            >
              {opt.label}
            </button>
          );
        }}
      </For>
    </div>
  );
}

export default Segmented;
