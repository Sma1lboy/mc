import { JSX, splitProps } from "solid-js";

// 基础卡片样式(Blocky Craft):panel 底 + 凹陷倒角 + 直角 + 内距 + 过渡。
const CARD_BASE =
  "bg-panel shadow-sunken rounded-none p-4 " +
  "transition-[transform,box-shadow,background-color] duration-200 ease-app";
const CARD_HOVER = "cursor-pointer hover:bg-panel-2 hover:-translate-y-px active:translate-y-0";

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
