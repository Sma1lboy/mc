import { JSX, splitProps } from "solid-js";

/* ============================================================================
 * Tag —— Blocky Craft 静态标签(类别 / 加载器:Fabric、Optimization…)。
 * 非交互;沙金文字 + panel-2 底,10px,直角。可点的筛选项用 <Chip>。
 * ========================================================================== */

export interface TagProps {
  children: JSX.Element;
  class?: string;
  title?: string;
}

export function Tag(props: TagProps): JSX.Element {
  const [local, rest] = splitProps(props, ["children", "class"]);
  return (
    <span
      {...rest}
      class={`inline-flex items-center bg-panel-2 text-tag text-[10px] leading-none px-[8px] py-[3px] rounded-none whitespace-nowrap${
        local.class ? " " + local.class : ""
      }`}
    >
      {local.children}
    </span>
  );
}

export default Tag;
