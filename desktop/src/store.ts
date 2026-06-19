import { createSignal } from "solid-js";

// 应用的页面标识。Rail 左图标栏的主导航即对应这几个值。
export type Page = "home" | "library" | "discover" | "settings";

/**
 * 全局轻量状态:用模块级 createSignal 暴露,任何组件 import 即可读写,
 * 无需 Context Provider 包裹。SolidJS 的信号本身就是细粒度响应式的,
 * 在模块作用域创建一次、全程复用即可达到「全局 store」的效果。
 */

// 当前页面(默认主页 dashboard)。
export const [currentPage, setCurrentPage] = createSignal<Page>("home");

/**
 * 当前选中的游戏根目录(GameRoot.path)。
 * null 表示尚未选择/尚未加载完成;实例列表等数据均以它为查询参数。
 */
export const [currentRoot, setCurrentRoot] = createSignal<string | null>(null);
