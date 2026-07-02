import { useEffect, useRef } from "react";
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
  className?: string;
}

export function SearchBox(props: SearchBoxProps): React.ReactElement {
  const inputEl = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (props.autofocus) inputEl.current?.focus();
    // 挂载即聚焦(与 Solid onMount 等价);autofocus 在生命周期内稳定。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div
      className={
        // 容器(Blocky):满宽、石质暗底深凹倒角、直角、高 36px;聚焦内含子元素时 accent 外描边。
        "inline-flex items-center gap-[8px] w-full bg-sidebar shadow-input " +
        "rounded-none px-[12px] h-[36px] " +
        "transition-[box-shadow] duration-[var(--dur)] ease-app " +
        "focus-within:ring-2 focus-within:ring-accent" +
        (props.className ? " " + props.className : "")
      }
    >
      {/* 放大镜图标 (内联 SVG, currentColor 跟随 dim 文字色)。 */}
      <svg
        className="shrink-0 text-dim"
        width="16"
        height="16"
        viewBox="0 0 16 16"
        fill="none"
        aria-hidden="true"
      >
        <circle cx="7" cy="7" r="4.5" stroke="currentColor" strokeWidth="1.6" />
        <path
          d="m11 11 3 3"
          stroke="currentColor"
          strokeWidth="1.6"
          strokeLinecap="round"
        />
      </svg>

      <input
        ref={inputEl}
        // 输入框:占满剩余、可收缩、无边框/描边、透明底、主文字色、基础字号、
        // 继承字体;placeholder 用 dim 文字色。
        className="flex-1 min-w-0 border-none bg-transparent text-fg text-[var(--fs-base)] font-[inherit] placeholder:text-faint focus-visible:outline-none"
        type="text"
        name={props.name ?? "search"}
        autoComplete="off"
        spellCheck={false}
        aria-label={props.label ?? props.placeholder ?? t("components.searchBox.label")}
        value={props.value}
        placeholder={props.placeholder ?? t("components.searchBox.placeholder")}
        // React onChange = input 语义(每次击键),对齐 Solid onInput;上抛字符串。
        onChange={(e) => props.onInput(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") props.onEnter?.(props.value);
          else if (e.key === "Escape" && props.onEscape) {
            e.preventDefault();
            props.onEscape();
          }
        }}
      />

      {/* 有内容时显示清除按钮。 */}
      {props.value.length > 0 && (
        <button
          type="button"
          // 清除按钮:18x18 居中、无边框透明底、dim 文字色、指针、xs 圆角、
          // 颜色+背景过渡;hover 转主文字色 + n-6 底。
          className="shrink-0 inline-flex items-center justify-center w-[18px] h-[18px] border-none bg-transparent text-dim cursor-pointer rounded-none transition-[color,background-color] duration-[var(--dur)] ease-app hover:text-fg hover:bg-panel-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          aria-label={t("components.searchBox.clear")}
          onClick={() => props.onInput("")}
        >
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
            <path
              d="M3 3l6 6M9 3l-6 6"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            />
          </svg>
        </button>
      )}
    </div>
  );
}

export default SearchBox;
