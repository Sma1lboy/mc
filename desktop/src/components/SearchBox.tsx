import { JSX, Show, onMount } from "solid-js";
import { t } from "../i18n";

// SearchBox —— 圆角搜索框, 背景 n-5, 带放大镜图标。
// props 契约:
//   value: 当前文本 (受控)
//   onInput: 文本变化回调 (传入新字符串)
//   placeholder?: 占位文案
export interface SearchBoxProps {
  value: string;
  onInput: (value: string) => void;
  placeholder?: string;
  label?: string;
  name?: string;
  /** 回车回调 (如触发搜索)。 */
  onEnter?: (value: string) => void;
  /** Esc 回调 (如清空 / 退出浏览)。 */
  onEscape?: () => void;
  /** 挂载即聚焦输入框 (进入浏览/添加模式时直接可打字)。 */
  autofocus?: boolean;
  class?: string;
}

export function SearchBox(props: SearchBoxProps): JSX.Element {
  let inputEl: HTMLInputElement | undefined;
  onMount(() => {
    if (props.autofocus) inputEl?.focus();
  });
  return (
    <div
      class={
        // 容器:inline-flex 居中、间距 8px、满宽、n-5 背景、透明边、控件圆角、
        // 左右内距 12px、高 36px、边色+背景过渡;聚焦内含子元素时 accent 描边。
        "inline-flex items-center gap-[8px] w-full glass-input " +
        "rounded-ctl px-[12px] h-[36px] " +
        "transition-[border-color,background-color,box-shadow] duration-[var(--dur)] ease-app " +
        "focus-within:border-a-4 focus-within:ring-2 focus-within:ring-a-5/30" +
        (props.class ? " " + props.class : "")
      }
    >
      {/* 放大镜图标 (内联 SVG, currentColor 跟随 dim 文字色)。 */}
      <svg
        class="shrink-0 text-dim"
        width="16"
        height="16"
        viewBox="0 0 16 16"
        fill="none"
        aria-hidden="true"
      >
        <circle cx="7" cy="7" r="4.5" stroke="currentColor" stroke-width="1.6" />
        <path
          d="m11 11 3 3"
          stroke="currentColor"
          stroke-width="1.6"
          stroke-linecap="round"
        />
      </svg>

      <input
        ref={inputEl}
        // 输入框:占满剩余、可收缩、无边框/描边、透明底、主文字色、基础字号、
        // 继承字体;placeholder 用 dim 文字色。
        class="flex-1 min-w-0 border-none bg-transparent text-fg text-[var(--fs-base)] font-[inherit] placeholder:text-dim focus-visible:outline-none"
        type="text"
        name={props.name ?? "search"}
        autocomplete="off"
        spellcheck={false}
        aria-label={props.label ?? props.placeholder ?? t("components.searchBox.label")}
        value={props.value}
        placeholder={props.placeholder ?? t("components.searchBox.placeholder")}
        // SolidJS: 用原生 input 事件读取 value, 上抛字符串。
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") props.onEnter?.(props.value);
          else if (e.key === "Escape" && props.onEscape) {
            e.preventDefault();
            props.onEscape();
          }
        }}
      />

      {/* 有内容时显示清除按钮。 */}
      <Show when={props.value.length > 0}>
        <button
          type="button"
          // 清除按钮:18x18 居中、无边框透明底、dim 文字色、指针、xs 圆角、
          // 颜色+背景过渡;hover 转主文字色 + n-6 底。
          class="shrink-0 inline-flex items-center justify-center w-[18px] h-[18px] border-none bg-transparent text-dim cursor-pointer rounded-xs transition-[color,background-color] duration-[var(--dur)] ease-app hover:text-fg hover:bg-glass-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
          aria-label={t("components.searchBox.clear")}
          onClick={() => props.onInput("")}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
            <path
              d="M3 3l6 6M9 3l-6 6"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
            />
          </svg>
        </button>
      </Show>
    </div>
  );
}

export default SearchBox;
