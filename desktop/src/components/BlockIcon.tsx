import { JSX } from "solid-js";

/* ============================================================================
 * BlockIcon —— Blocky Craft 草方块占位(Logo / 简单实例占位)。
 * 上草绿 / 下泥棕双色块 + 凸起倒角,直角方形。尺寸由调用方 class 决定(w/h)。
 * 需要确定性像素图标(identicon 风)的实例图标用 <InstanceIcon>。
 * ========================================================================== */

export interface BlockIconProps {
  class?: string;
  title?: string;
  style?: JSX.CSSProperties | string;
}

export function BlockIcon(props: BlockIconProps): JSX.Element {
  return (
    <div
      title={props.title}
      class={`shadow-raised rounded-none${props.class ? " " + props.class : ""}`}
      style={
        typeof props.style === "string"
          ? props.style
          : {
              background:
                "linear-gradient(var(--grass-top) 0 42%, var(--grass-side) 42% 100%)",
              ...(props.style ?? {}),
            }
      }
    />
  );
}

export default BlockIcon;
