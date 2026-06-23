import { JSX, splitProps } from "solid-js";

// 基础样式(Tailwind 内联):行内 flex 居中、14px 基准字号、控件圆角、~240ms 过渡。
const BTN_BASE =
  "inline-flex items-center justify-center gap-[6px] " +
  "text-[length:var(--fs-base)] font-[inherit] font-medium leading-none " +
  "px-[14px] py-[8px] border border-transparent rounded-ctl " +
  "cursor-pointer select-none whitespace-nowrap " +
  "transition-[background-color,border-color,color,opacity,transform] duration-[var(--dur)] ease-app " +
  // 轻微下压反馈 + 禁用态。
  "active:enabled:translate-y-px disabled:opacity-45 disabled:cursor-not-allowed";

// 三种变体:primary(accent 实心白字) / ghost(透明底 hover 中性灰) / danger(红色破坏性)。
const BTN_VARIANT: Record<string, string> = {
  primary: "bg-a-4 text-white hover:enabled:bg-a-5 active:enabled:bg-a-3",
  ghost:
    "bg-transparent text-fg hover:enabled:bg-glass-hover active:enabled:bg-n-6",
  danger:
    "bg-danger text-white hover:enabled:bg-danger-hover active:enabled:bg-danger-hover",
};

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
      class={`${BTN_BASE} ${BTN_VARIANT[local.variant ?? "primary"]}${
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
