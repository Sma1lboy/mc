import type { CSSProperties, ElementType, ReactNode } from "react";
import clsx from "clsx";

/* ============================================================================
 * Panel —— Blocky Craft 倒角表面原语(方块工坊的「灵魂」)。
 *
 * MC GUI 风格的立体边:用 inset 双向阴影模拟,不用圆角。四种变体:
 *   sunken  凹陷 —— 卡片 / 面板(默认)
 *   raised  凸起 —— 按钮 / 高亮块
 *   pressed 按下 —— 上下翻转(按下感)
 *   input   深凹 —— 输入框 / 分段控件轨
 * stone 叠石质纹理(侧栏 / 大面板)。底色默认 --panel,stone 时用 --bg-sidebar。
 * 通过 `as` 渲染任意标签;额外类名/样式/事件透传。
 * ========================================================================== */

const BEVEL: Record<string, string> = {
  sunken: "shadow-sunken",
  raised: "shadow-raised",
  pressed: "shadow-pressed",
  input: "shadow-input",
};

export interface PanelProps {
  /** 倒角变体,默认 sunken。 */
  variant?: "sunken" | "raised" | "pressed" | "input";
  /** 叠石质纹理(侧栏 / 大面板),底色转 --bg-sidebar。 */
  stone?: boolean;
  /** 渲染的标签名,默认 div。 */
  as?: string;
  children?: ReactNode;
  className?: string;
  style?: CSSProperties;
  onClick?: (e: MouseEvent) => void;
  title?: string;
}

export function Panel(props: PanelProps): React.ReactElement {
  const { variant, stone, as, children, className, style, onClick, title } = props;
  const Comp = (as ?? "div") as ElementType;

  return (
    <Comp
      style={style}
      title={title}
      className={clsx(stone ? "stone" : "bg-panel", BEVEL[variant ?? "sunken"], className)}
      onClick={(e: React.MouseEvent) => onClick?.(e.nativeEvent)}
    >
      {children}
    </Comp>
  );
}

export default Panel;
