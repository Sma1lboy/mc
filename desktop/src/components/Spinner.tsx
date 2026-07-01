import { useEffect, useState, type CSSProperties } from "react";
import { t } from "../i18n";

// Spinner —— 盲文风 loading:按帧循环 10 个盲文点阵字符,形成连续旋转。
// 用 state + interval 逐帧切换(WebKit 不支持动画 content 属性,故走 JS)。
// props:
//   size?: 字号 px, 默认 20
//   className?: 额外类名 (定位用)
//   label?: 无障碍标签, 默认 "Loading"。
export interface SpinnerProps {
  size?: number;
  className?: string;
  label?: string;
}

const FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

export function Spinner(props: SpinnerProps): React.ReactElement {
  const size = props.size ?? 20;
  const [frame, setFrame] = useState(0);

  useEffect(() => {
    const timer = setInterval(() => setFrame((i) => (i + 1) % FRAMES.length), 80);
    return () => clearInterval(timer);
  }, []);

  return (
    <span
      className={`inline-flex shrink-0 items-center justify-center leading-none${
        props.className ? " " + props.className : ""
      }`}
      style={{
        width: `${size}px`,
        height: `${size}px`,
        fontSize: `${size}px`,
        color: "var(--a-5)",
      } as CSSProperties}
      role="status"
      aria-label={props.label ?? t("components.spinner.loading")}
    >
      {FRAMES[frame]}
    </span>
  );
}

export default Spinner;
