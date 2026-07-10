import { ACCENT_BTN_COMPACT } from "../styles";
import { t } from "../../i18n";
import type { PackKind } from "../../ipc/types";

export const FIELD =
  "bg-sidebar shadow-input h-[34px] px-[12px] rounded-none text-fg text-[13px] " +
  "placeholder:text-faint transition-[box-shadow] duration-150 focus-visible:outline-none " +
  "focus-visible:ring-2 focus-visible:ring-accent";
export const LABEL = "text-[12px] text-sub";
export const TAB =
  "px-[14px] py-[7px] text-[13px] font-semibold cursor-pointer border-b-2 border-b-transparent " +
  "text-muted hover:text-fg transition-colors duration-150";
export const TAB_ACTIVE = "!text-accent !border-b-accent";

export type InstanceManageTab =
  | "realm"
  | "overview"
  | "settings"
  | "mods"
  | "resource_pack"
  | "shader"
  | "datapack"
  | "worlds"
  | "servers"
  | "screenshots";

export const TABS = (): { key: InstanceManageTab; label: string }[] => [
  { key: "settings", label: t("instance.tabSettings") },
  { key: "mods", label: t("instance.tabMods") },
  { key: "resource_pack", label: t("instance.tabResourcePack") },
  { key: "shader", label: t("instance.tabShader") },
  { key: "datapack", label: t("instance.tabDatapack") },
  { key: "worlds", label: t("instance.tabWorlds") },
  { key: "servers", label: t("instance.tabServers") },
  { key: "screenshots", label: t("instance.tabScreenshots") },
];

export const isPackTab = (tab: InstanceManageTab): tab is PackKind =>
  tab === "resource_pack" || tab === "shader" || tab === "datapack";

/** 人类可读的字节大小;0 / 缺省返回空串。 */
export function fmtSize(bytes: number): string {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i += 1;
  }
  return `${n.toFixed(i > 0 && n < 10 ? 1 : 0)} ${units[i]}`;
}

export const INSTALL_BTN = ACCENT_BTN_COMPACT;
export const DEL_BTN =
  "shrink-0 text-[12px] text-danger-text px-[8px] py-[4px] rounded-none cursor-pointer hover:bg-danger-soft";
export const OPEN_BTN =
  "shrink-0 text-[12px] text-muted px-[8px] py-[3px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2";
