import type { ReactNode } from "react";

/* ============================================================================
 * Tag —— Blocky Craft 静态标签(类别 / 加载器:Fabric、Optimization…)。
 * 非交互;沙金文字 + panel-2 底,10px,直角。可点的筛选项用 <Chip>。
 * ========================================================================== */

export interface TagProps {
  children: ReactNode;
  className?: string;
  title?: string;
}

export function Tag(props: TagProps): React.ReactElement {
  const { children, className, title } = props;
  return (
    <span
      title={title}
      className={`inline-flex items-center bg-panel-2 text-tag text-[10px] leading-none px-[8px] py-[3px] rounded-none whitespace-nowrap${
        className ? " " + className : ""
      }`}
    >
      {children}
    </span>
  );
}

export default Tag;
