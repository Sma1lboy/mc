import { createSignal } from "solid-js";
import { applyTheme } from "./theme/theme";

// 页面标识。Modrinth 布局用 home/library/discover/settings;
// PCL 布局用 launch/discover/settings/more(顶部 Tab,带图标)。
export type Page = "home" | "library" | "discover" | "settings" | "launch" | "more";

/** 界面布局风格:Modrinth(深色三区)或 PCL(浅色顶栏 Tab)。 */
export type LayoutMode = "modrinth" | "pcl";

/**
 * 全局轻量状态:模块级 createSignal,任何组件 import 即读写,无需 Context。
 */

// 当前页面(Modrinth 默认 dashboard;切到 PCL 时改成 launch)。
export const [currentPage, setCurrentPage] = createSignal<Page>("home");

/** 当前选中的游戏根目录(GameRoot.path);null = 未选/未加载。 */
export const [currentRoot, setCurrentRoot] = createSignal<string | null>(null);

/** 界面布局,默认 Modrinth 风。 */
export const [layoutMode, setLayoutMode] = createSignal<LayoutMode>("pcl");

/**
 * 切换布局,并套用与该布局相称的主题预设,让两种风格各自"对味":
 *   - PCL:浅色 + 蓝(PCL 招牌)
 *   - Modrinth:深色 + 绿
 * 用户之后仍可在设置里单独微调主题色。
 */
export function switchLayout(mode: LayoutMode) {
  setLayoutMode(mode);
  if (mode === "pcl") {
    applyTheme({ mode: "light", hue: 214, saturation: 88, lightness: 52 });
    setCurrentPage("launch");
  } else {
    applyTheme({ mode: "dark", hue: 150, saturation: 60, lightness: 45 });
    setCurrentPage("home");
  }
}
