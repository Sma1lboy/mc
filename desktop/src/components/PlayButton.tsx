import { JSX, Show } from "solid-js";
import "./PlayButton.css";

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
      class={`ui-play${props.running ? " ui-play--running" : ""}`}
      disabled={props.disabled}
      onClick={(e) => {
        if (props.disabled) return;
        props.onClick?.(e);
      }}
      title={props.running ? "Stop" : "Play"}
    >
      <Show
        when={props.running}
        fallback={
          // ▶ 播放三角 (内联 SVG, 用 currentColor 跟随文字色)。
          <svg
            class="ui-play__icon"
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
          class="ui-play__icon ui-play__spin"
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
