import { JSX } from "solid-js";
import "./Spinner.css";

// Spinner —— accent 色旋转 loading。用于加载态占位。
// props:
//   size?: 直径 px, 默认 20
//   class?: 额外类名 (定位用)
export interface SpinnerProps {
  size?: number;
  class?: string;
  /** 无障碍标签, 默认 "Loading"。 */
  label?: string;
}

export function Spinner(props: SpinnerProps): JSX.Element {
  const size = () => props.size ?? 20;
  // 线宽随尺寸缩放, 小尺寸不至于太粗。
  const stroke = () => Math.max(2, size() / 10);
  const r = () => (size() - stroke()) / 2;
  const c = () => size() / 2;

  return (
    <svg
      class={`ui-spinner${props.class ? " " + props.class : ""}`}
      width={size()}
      height={size()}
      viewBox={`0 0 ${size()} ${size()}`}
      fill="none"
      role="status"
      aria-label={props.label ?? "Loading"}
    >
      {/* 底环 */}
      <circle
        class="ui-spinner__track"
        cx={c()}
        cy={c()}
        r={r()}
        stroke-width={stroke()}
      />
      {/* 高亮弧: 约占 1/4 周长 (用 dasharray 实现)。 */}
      <circle
        class="ui-spinner__head"
        cx={c()}
        cy={c()}
        r={r()}
        stroke-width={stroke()}
        stroke-linecap="round"
        stroke-dasharray={`${(2 * Math.PI * r()) / 4} ${2 * Math.PI * r()}`}
      />
    </svg>
  );
}

export default Spinner;
