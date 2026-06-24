import { JSX, splitProps } from "solid-js";
import { Dynamic } from "solid-js/web";

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
  children?: JSX.Element;
  class?: string;
  /** Solid classList(条件类),与 class 共存。 */
  classList?: Record<string, boolean | undefined>;
  style?: JSX.CSSProperties | string;
  onClick?: (e: MouseEvent) => void;
  title?: string;
}

export function Panel(props: PanelProps): JSX.Element {
  const [local, rest] = splitProps(props, [
    "variant",
    "stone",
    "as",
    "children",
    "class",
    "onClick",
  ]);

  return (
    <Dynamic
      component={local.as ?? "div"}
      {...rest}
      class={`${local.stone ? "stone" : "bg-panel"} ${BEVEL[local.variant ?? "sunken"]}${
        local.class ? " " + local.class : ""
      }`}
      onClick={(e: MouseEvent) => local.onClick?.(e)}
    >
      {local.children}
    </Dynamic>
  );
}

export default Panel;
