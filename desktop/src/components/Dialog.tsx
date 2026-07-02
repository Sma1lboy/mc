import type { ReactNode } from "react";
import { Dialog as Ark } from "@ark-ui/react/dialog";
import { Portal } from "@ark-ui/react/portal";

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
  children: ReactNode;
  /** Content 卡片额外类名(尺寸/覆盖);不透明面板皮肤(底/边/倒角)已恒定应用,这里只加尺寸等。 */
  contentClass?: string;
  /** 遮罩类名(默认深色半透明)。 */
  backdropClass?: string;
  /** 无障碍标题(aria-label)。 */
  label?: string;
}

export function Dialog({ open, onClose, children, contentClass, backdropClass, label }: DialogProps) {
  return (
    <Ark.Root
      open={open}
      onOpenChange={(e: { open: boolean }) => {
        if (!e.open) onClose();
      }}
    >
      <Portal>
        <Ark.Backdrop className={"fixed inset-0 z-[100] " + (backdropClass ?? "bg-[rgba(8,7,5,0.62)]")} />
        <Ark.Positioner className="fixed inset-0 z-[100] flex items-center justify-center p-[24px] overscroll-contain">
          <Ark.Content
            aria-label={label}
            // 不透明面板皮肤恒定应用(避免调用方只传尺寸 contentClass 时漏掉底色导致弹窗透明);
            // contentClass 仅追加尺寸/覆盖。
            className={
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent " +
              "bg-panel text-fg border border-titlebar shadow-raised rounded-none overflow-hidden " +
              (contentClass ?? "w-[420px] max-w-[calc(100vw-48px)]")
            }
          >
            {children}
          </Ark.Content>
        </Ark.Positioner>
      </Portal>
    </Ark.Root>
  );
}

export default Dialog;
