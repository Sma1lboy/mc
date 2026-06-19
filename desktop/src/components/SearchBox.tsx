import { JSX, Show } from "solid-js";
import "./SearchBox.css";

// SearchBox —— 圆角搜索框, 背景 n-5, 带放大镜图标。
// props 契约:
//   value: 当前文本 (受控)
//   onInput: 文本变化回调 (传入新字符串)
//   placeholder?: 占位文案
export interface SearchBoxProps {
  value: string;
  onInput: (value: string) => void;
  placeholder?: string;
  /** 回车回调 (如触发搜索)。 */
  onEnter?: (value: string) => void;
  class?: string;
}

export function SearchBox(props: SearchBoxProps): JSX.Element {
  return (
    <div class={`ui-search${props.class ? " " + props.class : ""}`}>
      {/* 放大镜图标 (内联 SVG, currentColor 跟随 dim 文字色)。 */}
      <svg
        class="ui-search__icon"
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
        class="ui-search__input"
        type="text"
        value={props.value}
        placeholder={props.placeholder ?? "Search"}
        // SolidJS: 用原生 input 事件读取 value, 上抛字符串。
        onInput={(e) => props.onInput(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") props.onEnter?.(props.value);
        }}
      />

      {/* 有内容时显示清除按钮。 */}
      <Show when={props.value.length > 0}>
        <button
          type="button"
          class="ui-search__clear"
          aria-label="Clear search"
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
