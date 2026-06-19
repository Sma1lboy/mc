import { JSX, splitProps } from "solid-js";
import "./Button.css";

// Button —— 通用按钮组件。
// props 契约 (页面 agent 按此调用):
//   variant?: 'primary' | 'ghost' | 'danger'  默认 'primary'
//   children: 按钮内容 (文字 / 图标)
//   onClick?: 点击回调
//   disabled?: 禁用态
export interface ButtonProps {
  variant?: "primary" | "ghost" | "danger";
  children: JSX.Element;
  onClick?: (e: MouseEvent) => void;
  disabled?: boolean;
  /** 透传原生属性 (如 title / aria-label / type), 不破坏既有 props 契约。 */
  title?: string;
  type?: "button" | "submit" | "reset";
  class?: string;
}

export function Button(props: ButtonProps): JSX.Element {
  // splitProps 保持响应式: 不要解构 props (SolidJS 会丢失响应性)。
  const [local, rest] = splitProps(props, [
    "variant",
    "children",
    "onClick",
    "disabled",
    "class",
  ]);

  return (
    <button
      {...rest}
      type={props.type ?? "button"}
      class={`ui-btn ui-btn--${local.variant ?? "primary"}${
        local.class ? " " + local.class : ""
      }`}
      disabled={local.disabled}
      onClick={(e) => {
        // 禁用时不触发 (双保险, 原生 disabled 已拦截大部分)。
        if (local.disabled) return;
        local.onClick?.(e);
      }}
    >
      {local.children}
    </button>
  );
}

export default Button;
