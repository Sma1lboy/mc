import type { ReactNode } from "react";
import { Tooltip as Ark } from "@ark-ui/react/tooltip";
import { Portal } from "@ark-ui/react/portal";

/**
 * Tooltip —— 基于 Ark UI(headless)的house-styled 悬浮提示。
 * 定位/延迟/键盘可达由 Ark 负责;气泡用我们的令牌着色(反色气泡,深浅主题都有对比)。
 */
export interface TooltipProps {
  /** 提示内容(文本或任意节点)。 */
  content: ReactNode;
  /** 触发元素(图标/按钮等);Trigger 自身渲染成 button。 */
  children: ReactNode;
  placement?: "top" | "bottom" | "left" | "right";
  openDelay?: number;
}

export function Tooltip({ content, children, placement, openDelay }: TooltipProps) {
  return (
    <Ark.Root
      openDelay={openDelay ?? 300}
      closeDelay={80}
      positioning={{ placement: placement ?? "top" }}
    >
      <Ark.Trigger className="inline-flex items-center bg-transparent border-none p-0 cursor-help text-muted hover:text-fg transition-colors duration-150">
        {children}
      </Ark.Trigger>
      <Portal>
        <Ark.Positioner>
          <Ark.Content className="z-[300] max-w-[280px] px-[10px] py-[6px] rounded-none border border-titlebar bg-panel-2 shadow-raised text-fg text-[12px] leading-[1.5]">
            {content}
          </Ark.Content>
        </Ark.Positioner>
      </Portal>
    </Ark.Root>
  );
}

export default Tooltip;
