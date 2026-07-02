import type { ReactNode } from "react";

/* ============================================================================
 * NavItem —— Blocky Craft 侧栏导航项(首页 / 发现 / 库 / 设置)。
 * 42×42 命中区;选中 = 熔岩橙凸起方块 + 白描边图标;未选 = 灰图标 hover 提亮。
 * 图标由调用方以 children 传入(线性 stroke 2.2 的内联 SVG)。
 * ========================================================================== */

export interface NavItemProps {
  active?: boolean;
  onClick?: (e: MouseEvent) => void;
  title?: string;
  /** 图标(内联 SVG,fill=none stroke=currentColor)。 */
  children: ReactNode;
}

export function NavItem(props: NavItemProps): React.ReactElement {
  return (
    <button
      type="button"
      title={props.title}
      aria-label={props.title}
      aria-current={props.active ? "page" : undefined}
      onClick={(e) => props.onClick?.(e.nativeEvent)}
      className={`inline-flex items-center justify-center h-[42px] w-[42px] rounded-none border-none cursor-pointer transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent ${
        props.active
          ? "bg-accent text-white shadow-raised"
          : "bg-transparent text-[#9a9a86] hover:bg-panel-2 hover:text-fg"
      }`}
    >
      {props.children}
    </button>
  );
}

export default NavItem;
