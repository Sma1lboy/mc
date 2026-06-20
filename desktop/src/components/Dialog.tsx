import { Component, JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { Dialog as Ark } from "@ark-ui/solid/dialog";

/**
 * Dialog —— 基于 Ark UI(headless)的模态弹窗外壳:焦点陷阱 / Esc 关闭 /
 * 点遮罩关闭 / 滚动锁 / ARIA 都由 Ark 负责。视觉(遮罩、卡片)用我们的
 * Tailwind 令牌;调用方把内容塞进 children,用 contentClass 定制卡片尺寸/皮肤。
 *
 * 受控:open 为 true 时显示;关闭(Esc/遮罩/调用方按钮)走 onClose。
 */
export interface DialogProps {
  open: boolean;
  onClose: () => void;
  children: JSX.Element;
  /** Content 卡片类名(尺寸/背景/圆角);默认给一个中性卡片。 */
  contentClass?: string;
  /** 遮罩类名(默认深色半透明 + 亚克力模糊);PCL 传 "pcl-dlg-mask" 复用其逃生口。 */
  backdropClass?: string;
  /** 无障碍标题(aria-label)。 */
  label?: string;
}

export const Dialog: Component<DialogProps> = (props) => {
  return (
    <Ark.Root
      open={props.open}
      onOpenChange={(e: { open: boolean }) => {
        if (!e.open) props.onClose();
      }}
    >
      <Portal>
        <Ark.Backdrop
          class={
            "fixed inset-0 z-[100] " +
            (props.backdropClass ?? "bg-[rgba(8,10,14,0.5)] backdrop-blur-[var(--blur-r)]")
          }
        />
        <Ark.Positioner class="fixed inset-0 z-[100] flex items-center justify-center p-[24px]">
          <Ark.Content
            aria-label={props.label}
            class={
              props.contentClass ??
              "w-[420px] max-w-[calc(100vw-48px)] bg-card rounded-card shadow-card overflow-hidden focus:outline-none"
            }
          >
            {props.children}
          </Ark.Content>
        </Ark.Positioner>
      </Portal>
    </Ark.Root>
  );
};

export default Dialog;
