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

export { SearchBox } from "./SearchBox";
export type { SearchBoxProps } from "./SearchBox";

export { Icon } from "./Icon";
export type { IconProps, IconName } from "./Icon";

export { Avatar, STEVE_AVATAR, headUrl } from "./Avatar";

// 复合行/卡片 (Modrinth 风格)
export { InstanceRow } from "./InstanceRow";
export type { InstanceRowProps, InstanceRowData } from "./InstanceRow";

export { ModpackCard } from "./ModpackCard";
export type { ModpackCardProps, ModpackHit } from "./ModpackCard";

// Toast 通道
export { toast, ToastContainer } from "./Toast";
export type { ToastType, ToastOptions } from "./Toast";

// 格式化工具
export { formatRelativeTime, formatCount } from "./format";
