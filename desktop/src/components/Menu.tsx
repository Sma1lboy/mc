import type { ReactNode } from "react";
import { Menu as Ark } from "@ark-ui/react/menu";
import { Portal } from "@ark-ui/react/portal";

/**
 * Menu —— 基于 Ark UI(headless)的house-styled 菜单 seam,与 Select/Tooltip/Dialog
 * 同一套封装风格。把**菜单外壳**(Portal + 定位 + Content 盒子样式)集中一处,
 * 调用方只组合自带样式的 Trigger + 任意 Item 内容。
 *
 *   Menu.Root / Menu.Trigger —— 直接透传 Ark(各处触发器样式不同,自带 class)。
 *   Menu.Content             —— 已封装 Portal + Positioner + 着色的内容盒。
 *   Menu.Item                —— 简单文字项(InstanceRow 那种),已着色 + data-[highlighted]。
 *   Menu.ItemRaw             —— 透传 Ark.Item,供需要富内容/自定义排版的项(账号行)用,
 *                               让所有 @ark-ui 访问都收敛在本 seam 内。
 *   Menu.Separator           —— 着色分隔线。
 */

// 简单文字项的交互/着色基样式。
const ITEM_NORMAL =
  "flex items-center px-[10px] py-[7px] rounded-none cursor-pointer select-none " +
  "text-fg data-[highlighted]:bg-panel-3 data-[highlighted]:text-accent motion-reduce:transition-none";
const ITEM_DANGER =
  "flex items-center px-[10px] py-[7px] rounded-none cursor-pointer select-none " +
  "text-danger-text data-[highlighted]:bg-danger-soft motion-reduce:transition-none";

interface MenuContentProps {
  children: ReactNode;
  /** 追加类名(如收窄/加宽);默认外壳已含 z/边框/圆角/底色/阴影。 */
  className?: string;
}
function MenuContent({ children, className }: MenuContentProps) {
  return (
    <Portal>
      <Ark.Positioner>
        <Ark.Content
          className={
            "z-[300] min-w-[168px] p-[4px] rounded-none bg-panel-2 text-fg border border-titlebar shadow-raised " +
            "flex flex-col gap-[2px] text-[13px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent " +
            (className ?? "")
          }
        >
          {children}
        </Ark.Content>
      </Ark.Positioner>
    </Portal>
  );
}

interface MenuItemProps {
  value: string;
  danger?: boolean;
  className?: string;
  children: ReactNode;
}
function MenuItem({ value, danger, className, children }: MenuItemProps) {
  return (
    <Ark.Item
      value={value}
      className={(danger ? ITEM_DANGER : ITEM_NORMAL) + (className ? " " + className : "")}
    >
      {children}
    </Ark.Item>
  );
}

function MenuSeparator({ className }: { className?: string }) {
  return <Ark.Separator className={"my-[4px] h-px bg-titlebar border-none " + (className ?? "")} />;
}

export const Menu = {
  Root: Ark.Root,
  Trigger: Ark.Trigger,
  Content: MenuContent,
  Item: MenuItem,
  ItemRaw: Ark.Item,
  Separator: MenuSeparator,
};
