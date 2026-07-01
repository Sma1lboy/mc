import type { CSSProperties } from "react";

/* ============================================================================
 * BlockIcon —— Blocky Craft 草方块占位(Logo / 简单实例占位)。
 * 上草绿 / 下泥棕双色块 + 凸起倒角,直角方形。尺寸由调用方 class 决定(w/h)。
 * 需要确定性像素图标(identicon 风)的实例图标用 <InstanceIcon>。
 * ========================================================================== */

export interface BlockIconProps {
  className?: string;
  title?: string;
  style?: CSSProperties;
}

export function BlockIcon(props: BlockIconProps): React.ReactElement {
  return (
    <div
      title={props.title}
      className={`shadow-raised rounded-none${props.className ? " " + props.className : ""}`}
      style={{
        background: "linear-gradient(var(--grass-top) 0 42%, var(--grass-side) 42% 100%)",
        ...(props.style ?? {}),
      }}
    />
  );
}

export default BlockIcon;
