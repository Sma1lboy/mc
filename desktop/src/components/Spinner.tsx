import { JSX } from "solid-js";
import "./Spinner.css"; // 残留 @keyframes(旋转动画)

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
      class={`inline-block shrink-0 ui-spinner-rotate${props.class ? " " + props.class : ""}`}
      width={size()}
      height={size()}
      viewBox={`0 0 ${size()} ${size()}`}
      fill="none"
      role="status"
      aria-label={props.label ?? "Loading"}
    >
      {/* 底环: 半透明中性色。 */}
      <circle
        class="[stroke:var(--n-6)]"
        cx={c()}
        cy={c()}
        r={r()}
        stroke-width={stroke()}
      />
      {/* 高亮弧: accent 色, 约占 1/4 周长 (dasharray)。 */}
      <circle
        class="[stroke:var(--a-5)]"
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
