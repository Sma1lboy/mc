import type { ReactNode } from "react";

// 基础样式(Blocky Craft):行内 flex 居中、方块直角、倒角立体边、按下翻转倒角。
const BTN_BASE =
  "inline-flex items-center justify-center gap-[6px] " +
  "text-[length:var(--fs-base)] font-[inherit] font-medium leading-none " +
  "px-[14px] py-[9px] border-none rounded-none " +
  "cursor-pointer select-none whitespace-nowrap " +
  "transition-[background-color,color,box-shadow,opacity] duration-[var(--dur)] ease-app " +
  // 按下态:倒角上下翻转(MC GUI 的「按下感」)+ 禁用态。
  "active:enabled:shadow-pressed disabled:opacity-50 disabled:cursor-not-allowed";

// 三种变体:primary(熔岩橙凸起) / ghost(panel-2 次按钮凸起) / danger(红色破坏性凸起)。
const BTN_VARIANT: Record<string, string> = {
  primary: "bg-accent text-white shadow-raised hover:enabled:bg-accent-hover",
  ghost: "bg-panel-3 text-fg shadow-raised hover:enabled:brightness-110",
  danger: "bg-danger text-white shadow-raised hover:enabled:bg-danger-hover",
};

// Button —— 通用按钮组件。
// props 契约 (页面 agent 按此调用):
//   variant?: 'primary' | 'ghost' | 'danger'  默认 'primary'
//   children: 按钮内容 (文字 / 图标)
//   onClick?: 点击回调
//   disabled?: 禁用态
export interface ButtonProps {
  variant?: "primary" | "ghost" | "danger";
  children: ReactNode;
  onClick?: (e: MouseEvent) => void;
  disabled?: boolean;
  /** 透传原生属性 (如 title / aria-label / type), 不破坏既有 props 契约。 */
  title?: string;
  type?: "button" | "submit" | "reset";
  className?: string;
}

export function Button(props: ButtonProps): React.ReactElement {
  const { variant, children, onClick, disabled, className, title, type } = props;

  return (
    <button
      title={title}
      type={type ?? "button"}
      className={`${BTN_BASE} ${BTN_VARIANT[variant ?? "primary"]}${
        className ? " " + className : ""
      }`}
      disabled={disabled}
      onClick={(e) => {
        // 禁用时不触发 (双保险, 原生 disabled 已拦截大部分)。
        if (disabled) return;
        onClick?.(e.nativeEvent);
      }}
    >
      {children}
    </button>
  );
}

export default Button;
