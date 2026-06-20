import { Component, JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { Tooltip as Ark } from "@ark-ui/solid/tooltip";

/**
 * Tooltip —— 基于 Ark UI(headless)的house-styled 悬浮提示。
 * 定位/延迟/键盘可达由 Ark 负责;气泡用我们的令牌着色(反色气泡,深浅主题都有对比)。
 */
export interface TooltipProps {
  /** 提示内容(文本或任意节点)。 */
  content: JSX.Element;
  /** 触发元素(图标/按钮等);Trigger 自身渲染成 button。 */
  children: JSX.Element;
  placement?: "top" | "bottom" | "left" | "right";
  openDelay?: number;
}

export const Tooltip: Component<TooltipProps> = (props) => {
  return (
    <Ark.Root
      openDelay={props.openDelay ?? 300}
      closeDelay={80}
      positioning={{ placement: props.placement ?? "top" }}
    >
      <Ark.Trigger class="inline-flex items-center bg-transparent border-none p-0 cursor-help text-dim hover:text-fg transition-colors duration-150">
        {props.children}
      </Ark.Trigger>
      <Portal>
        <Ark.Positioner>
          <Ark.Content class="z-[300] max-w-[280px] px-[10px] py-[6px] rounded-ctl bg-n-8 text-n-1 text-[12px] leading-[1.5] shadow-card">
            {props.content}
          </Ark.Content>
        </Ark.Positioner>
      </Portal>
    </Ark.Root>
  );
};

export default Tooltip;
