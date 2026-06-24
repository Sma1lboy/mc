import { createSignal, JSX, onCleanup } from "solid-js";
import { t } from "../i18n";

// Spinner —— 盲文风 loading:按帧循环 10 个盲文点阵字符,形成连续旋转。
// 用 signal + interval 逐帧切换(WebKit 不支持动画 content 属性,故走 JS)。
// props:
//   size?: 字号 px, 默认 20
//   class?: 额外类名 (定位用)
//   label?: 无障碍标签, 默认 "Loading"。
export interface SpinnerProps {
  size?: number;
  class?: string;
  label?: string;
}

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export function Spinner(props: SpinnerProps): JSX.Element {
  const size = () => props.size ?? 20;
  const [frame, setFrame] = createSignal(0);

  const timer = setInterval(
    () => setFrame((i) => (i + 1) % FRAMES.length),
    80,
  );
  onCleanup(() => clearInterval(timer));

  return (
    <span
      class={`inline-flex shrink-0 items-center justify-center leading-none${
        props.class ? " " + props.class : ""
      }`}
      style={{
        width: `${size()}px`,
        height: `${size()}px`,
        "font-size": `${size()}px`,
        color: "var(--a-5)",
      }}
      role="status"
      aria-label={props.label ?? t("components.spinner.loading")}
    >
      {FRAMES[frame()]}
    </span>
  );
}

export default Spinner;
