import { JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { Menu as Ark } from "@ark-ui/solid/menu";

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
  "flex items-center px-[10px] py-[7px] rounded-xs cursor-pointer select-none " +
  "text-fg data-[highlighted]:bg-n-5 motion-reduce:transition-none";
const ITEM_DANGER =
  "flex items-center px-[10px] py-[7px] rounded-xs cursor-pointer select-none " +
  "text-[#e5848a] data-[highlighted]:bg-[rgba(229,132,138,0.14)] motion-reduce:transition-none";

interface MenuContentProps {
  children: JSX.Element;
  /** 追加类名(如收窄/加宽);默认外壳已含 z/边框/圆角/底色/阴影。 */
  class?: string;
}
function MenuContent(props: MenuContentProps): JSX.Element {
  return (
    <Portal>
      <Ark.Positioner>
        <Ark.Content
          class={
            "z-[300] min-w-[168px] p-[4px] border border-n-6 rounded-ctl bg-card shadow-card " +
            "flex flex-col gap-[2px] text-[13px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5 " +
            (props.class ?? "")
          }
        >
          {props.children}
        </Ark.Content>
      </Ark.Positioner>
    </Portal>
  );
}

interface MenuItemProps {
  value: string;
  danger?: boolean;
  class?: string;
  children: JSX.Element;
}
function MenuItem(props: MenuItemProps): JSX.Element {
  return (
    <Ark.Item
      value={props.value}
      class={(props.danger ? ITEM_DANGER : ITEM_NORMAL) + (props.class ? " " + props.class : "")}
    >
      {props.children}
    </Ark.Item>
  );
}

function MenuSeparator(props: { class?: string }): JSX.Element {
  return <Ark.Separator class={"my-[4px] h-px bg-n-6 border-none " + (props.class ?? "")} />;
}

export const Menu = {
  Root: Ark.Root,
  Trigger: Ark.Trigger,
  Content: MenuContent,
  Item: MenuItem,
  ItemRaw: Ark.Item,
  Separator: MenuSeparator,
};
