import { JSX, Show } from "solid-js";
import { t } from "../i18n";
import "./PlayButton.css"; // 残留 @keyframes(旋转环动画 ui-play-spin)

// 按钮基础样式(Tailwind 内联):accent 主操作按钮。
//   - 数值严格对齐原 CSS: gap 7px / padding 9px 18px / font 600 / line-height 1。
//   - 颜色/圆角/缓动走令牌桥接;--fs-base / --dur 不在桥接表 -> 引用原变量。
//   - 白字 #ffffff 直接字面量。
const PLAY_BASE =
  "inline-flex items-center justify-center gap-[7px] " +
  "text-[var(--fs-base)] font-sans font-semibold leading-none " +
  "py-[9px] px-[18px] border-none rounded-ctl text-[#ffffff] " +
  "cursor-pointer select-none whitespace-nowrap bg-a-4 " +
  "transition-[background-color,transform,opacity] duration-[var(--dur)] ease-app " +
  "hover:not-disabled:bg-a-5 active:not-disabled:bg-a-3 active:not-disabled:translate-y-px " +
  "disabled:opacity-50 disabled:cursor-not-allowed";
// running 态:更深 accent 区分"正在运行";hover 偏红(danger 令牌)警示可点击停止。
const PLAY_RUNNING = "bg-a-2 hover:not-disabled:bg-danger";

// PlayButton —— 启动游戏的主操作按钮 (accent 色)。
// props 契约:
//   onClick?: 点击回调 (启动 / 停止)
//   running?: 是否正在运行。true 时显示 ■ 停止图标 + 旋转 loading 环, 文案变 "Running"
//   label?: 自定义文案, 默认 "Play" / running 时 "Running"
export interface PlayButtonProps {
  onClick?: (e: MouseEvent) => void;
  running?: boolean;
  label?: string;
  disabled?: boolean;
}

export function PlayButton(props: PlayButtonProps): JSX.Element {
  // 默认文案: 非运行 "Play", 运行中 "Running"。
  const label = () => props.label ?? (props.running ? "Running" : "Play");

  return (
    <button
      type="button"
      class={`${PLAY_BASE}${props.running ? " " + PLAY_RUNNING : ""}`}
      disabled={props.disabled}
      onClick={(e) => {
        if (props.disabled) return;
        props.onClick?.(e);
      }}
      title={props.running ? t("components.play.stop") : t("components.play.start")}
    >
      <Show
        when={props.running}
        fallback={
          // ▶ 播放三角 (内联 SVG, 用 currentColor 跟随文字色)。
          <svg
            class="block shrink-0"
            width="13"
            height="13"
            viewBox="0 0 12 12"
            fill="currentColor"
            aria-hidden="true"
          >
            <path d="M2.5 1.6c0-.5.55-.82.99-.57l6.9 3.97c.45.26.45.92 0 1.18l-6.9 3.97a.66.66 0 0 1-.99-.57V1.6Z" />
          </svg>
        }
      >
        {/* running 态: 旋转环 + 中心 ■ 方块, 表达"运行中且可停止"。 */}
        <svg
          class="block shrink-0 ui-play__spin"
          width="14"
          height="14"
          viewBox="0 0 16 16"
          fill="none"
          aria-hidden="true"
        >
          {/* 半透明底环 */}
          <circle
            cx="8"
            cy="8"
            r="6.2"
            stroke="currentColor"
            stroke-opacity="0.3"
            stroke-width="2"
          />
          {/* 高亮弧 (旋转) */}
          <path
            d="M8 1.8a6.2 6.2 0 0 1 6.2 6.2"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
          />
        </svg>
      </Show>
      <span>{label()}</span>
    </button>
  );
}

export default PlayButton;
