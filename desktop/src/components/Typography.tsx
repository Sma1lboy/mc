import type { CSSProperties, ElementType, ReactNode } from "react";

/* ============================================================================
 * Typography —— Blocky Craft 字体角色封装。
 *
 *   <Heading>     像素标题(Pixelify Sans):页面标题 / 区块标题 / 实例名 / 欢迎语。
 *                 size 控制字号档(page=27/section=21/sub=15/mini=14)。
 *   <PixelLabel>  点阵短词(Press Start 2P):PLAY / CONTINUE / 下载量 / Modrinth。
 *                 仅用于短拉丁词与数字,**不要**包中文或长句。
 *
 * 字号严格对齐设计稿。颜色默认主文字,可由 class 覆盖。
 * ========================================================================== */

const HEADING_SIZE: Record<string, string> = {
  page: "text-[27px] tracking-[0.5px]",
  section: "text-[21px] tracking-[0.5px]",
  sub: "text-[15px]",
  mini: "text-[14px] tracking-[0.5px]",
};

export interface HeadingProps {
  /** 字号档,默认 section。 */
  size?: "page" | "section" | "sub" | "mini";
  /** 渲染标签,默认 div(标题层级由页面语义决定,这里只管视觉)。 */
  as?: string;
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
  title?: string;
}

export function Heading(props: HeadingProps): React.ReactElement {
  const { size, as, children, className, style, title } = props;
  const Comp = (as ?? "div") as ElementType;
  return (
    <Comp
      style={style}
      title={title}
      className={`font-display font-medium leading-tight text-fg ${HEADING_SIZE[size ?? "section"]}${
        className ? " " + className : ""
      }`}
    >
      {children}
    </Comp>
  );
}

export interface PixelLabelProps {
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
  title?: string;
}

/** 点阵短词(Press Start 2P)。默认 10px;字号/颜色由 className 调。 */
export function PixelLabel(props: PixelLabelProps): React.ReactElement {
  const { children, className, style, title } = props;
  return (
    <span
      style={style}
      title={title}
      className={`font-pixel text-[10px] leading-none${className ? " " + className : ""}`}
    >
      {children}
    </span>
  );
}
