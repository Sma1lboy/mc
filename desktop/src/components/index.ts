// index.ts —— 组件库统一出口。页面只从 "@/components" 导入。
//
// 用法示例:
//   import { Button, Card, PlayButton, InstanceRow, ModpackCard,
//            SearchBox, Spinner, toast, ToastContainer,
//            formatRelativeTime, formatCount } from "@/components";

// 基础控件
export { Button } from "./Button";
export type { ButtonProps } from "./Button";

export { Card } from "./Card";
export type { CardProps } from "./Card";

export { PlayButton } from "./PlayButton";
export type { PlayButtonProps } from "./PlayButton";

export { Spinner } from "./Spinner";
export type { SpinnerProps } from "./Spinner";

export { EmptyState } from "./EmptyState";
export { ErrorState } from "./ErrorState";
export { Toggle } from "./Toggle";

export { SearchBox } from "./SearchBox";
export type { SearchBoxProps } from "./SearchBox";

export { Icon } from "./Icon";
export type { IconProps, IconName } from "./Icon";

export { Avatar, STEVE_AVATAR, headUrl } from "./Avatar";

export { default as Lightbox } from "./Lightbox";
export type { LightboxImage } from "./Lightbox";

// Ark UI 封装(headless 原语 + 我们的 Tailwind 令牌着色)
export { Select } from "./Select";
export type { SelectProps, SelectOption } from "./Select";
export { Tooltip } from "./Tooltip";
export type { TooltipProps } from "./Tooltip";
export { Dialog } from "./Dialog";
export type { DialogProps } from "./Dialog";
export { Menu } from "./Menu";
export { NewInstanceDialog } from "./NewInstanceDialog";
export { InstanceManageDialog } from "./InstanceManageDialog";
export { AccountDialog } from "./AccountDialog";
export { BlockedFilesDialog } from "./BlockedFilesDialog";
export { ImportModpackDialog } from "./ImportModpackDialog";

// 复合行/卡片
export { InstanceRow } from "./InstanceRow";
export type { InstanceRowProps, InstanceRowData } from "./InstanceRow";

export { ModpackCard } from "./ModpackCard";
export type { ModpackCardProps, ModpackHit } from "./ModpackCard";

export { ModpackListItem } from "./ModpackListItem";
export type { ModpackListItemProps } from "./ModpackListItem";

export { ContentBrowser } from "./ContentBrowser";
export type { ContentBrowserProps } from "./ContentBrowser";

// Toast 通道
export { toast, ToastContainer } from "./Toast";
export type { ToastType, ToastOptions } from "./Toast";

// 格式化工具
export { formatRelativeTime, formatCount } from "./format";
