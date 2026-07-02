import type { CSSProperties, ReactNode } from "react";

// 基础卡片样式(Blocky Craft):panel 底 + 凹陷倒角 + 直角 + 内距 + 过渡。
const CARD_BASE =
  "bg-panel shadow-sunken rounded-none p-4 " +
  "transition-[transform,box-shadow,background-color] duration-200 ease-app";
const CARD_HOVER = "cursor-pointer hover:bg-panel-2 hover:-translate-y-px active:translate-y-0";

// Card —— 通用卡片容器。
// props 契约:
//   children: 卡片内容
//   className?: 额外类名 (页面用来加 grid item / 自定义 padding 等)
//   onClick?: 点击回调 (可点卡片场景)
//   hover?: 是否启用 hover 上移 + 阴影加深动画
export interface CardProps {
  children: ReactNode;
  className?: string;
  onClick?: (e: MouseEvent) => void;
  hover?: boolean;
  /** 允许透传 style (页面偶尔需要内联 grid / 尺寸覆盖)。 */
  style?: CSSProperties;
  title?: string;
}

export function Card(props: CardProps): React.ReactElement {
  const { children, className, onClick, hover, style, title } = props;

  return (
    <div
      title={title}
      className={`${CARD_BASE}${hover ? " " + CARD_HOVER : ""}${
        className ? " " + className : ""
      }`}
      style={style}
      onClick={(e) => onClick?.(e.nativeEvent)}
      // 可点卡片提供键盘可达性。
      role={onClick ? "button" : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={(e) => {
        if (onClick && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onClick(e.nativeEvent as unknown as MouseEvent);
        }
      }}
    >
      {children}
    </div>
  );
}

export default Card;
