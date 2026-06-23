import { For, JSX } from "solid-js";
import "./Spinner.css"; // @keyframes(像素方块明灭链)

// Spinner —— 像素风 loading:8 个 accent 方块绕环,明灭呈彗尾旋转(crispEdges 保持像素硬边)。
// props:
//   size?: 直径 px, 默认 20
//   class?: 额外类名 (定位用)
//   label?: 无障碍标签, 默认 "Loading"。
export interface SpinnerProps {
  size?: number;
  class?: string;
  label?: string;
}

// 12×12 视图里 3×3 方块的 8 个环位(跳过中心),顺时针自左上起。
const CELLS: [number, number][] = [
  [0.5, 0.5],
  [4.5, 0.5],
  [8.5, 0.5],
  [8.5, 4.5],
  [8.5, 8.5],
  [4.5, 8.5],
  [0.5, 8.5],
  [0.5, 4.5],
];

export function Spinner(props: SpinnerProps): JSX.Element {
  const size = () => props.size ?? 20;
  return (
    <svg
      class={`inline-block shrink-0${props.class ? " " + props.class : ""}`}
      width={size()}
      height={size()}
      viewBox="0 0 12 12"
      role="status"
      aria-label={props.label ?? "Loading"}
      shape-rendering="crispEdges"
    >
      <For each={CELLS}>
        {(cell, i) => (
          <rect
            class="ui-pixel-cell"
            x={cell[0]}
            y={cell[1]}
            width="3"
            height="3"
            fill="var(--a-5)"
            style={{ "animation-delay": `${-(i() * 0.1)}s` }}
          />
        )}
      </For>
    </svg>
  );
}

export default Spinner;
