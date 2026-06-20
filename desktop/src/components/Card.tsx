import { JSX, splitProps } from "solid-js";

// 基础卡片样式(Tailwind 内联):背景/圆角/阴影/内距 + 颜色过渡。
const CARD_BASE =
  "bg-card rounded-card shadow-card p-4 border border-transparent " +
  "transition-[transform,box-shadow,background-color,border-color] duration-200 ease-app";
// hover 卡片:上移 + 阴影加深 + 轻边框,active 回落一点。
const CARD_HOVER =
  "cursor-pointer hover:-translate-y-[3px] " +
  "hover:shadow-[0_8px_24px_rgba(0,0,0,0.45)] hover:border-n-6 active:-translate-y-px";

// Card —— 通用卡片容器。
// props 契约:
//   children: 卡片内容
//   class?: 额外类名 (页面用来加 grid item / 自定义 padding 等)
//   onClick?: 点击回调 (可点卡片场景)
//   hover?: 是否启用 hover 上移 + 阴影加深动画
export interface CardProps {
  children: JSX.Element;
  class?: string;
  onClick?: (e: MouseEvent) => void;
  hover?: boolean;
  /** 允许透传 style (页面偶尔需要内联 grid / 尺寸覆盖)。 */
  style?: JSX.CSSProperties | string;
  title?: string;
}

export function Card(props: CardProps): JSX.Element {
  const [local, rest] = splitProps(props, [
    "children",
    "class",
    "onClick",
    "hover",
    "style",
  ]);

  return (
    <div
      {...rest}
      class={`${CARD_BASE}${local.hover ? " " + CARD_HOVER : ""}${
        local.class ? " " + local.class : ""
      }`}
      style={local.style}
      onClick={(e) => local.onClick?.(e)}
      // 可点卡片提供键盘可达性。
      role={local.onClick ? "button" : undefined}
      tabindex={local.onClick ? 0 : undefined}
      onKeyDown={(e) => {
        if (local.onClick && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          local.onClick(e as unknown as MouseEvent);
        }
      }}
    >
      {local.children}
    </div>
  );
}

export default Card;
