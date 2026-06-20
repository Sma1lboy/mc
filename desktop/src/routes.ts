import type { Component } from "solid-js";
import type { Page } from "./store";
import Home from "./pages/Home";
import Library from "./pages/Library";
import Discover from "./pages/Discover";
import Settings from "./pages/Settings";
import PclLaunch from "./pages/PclLaunch";
import PclMore from "./pages/PclMore";

/**
 * 页面路由表 —— 把「哪个 page 渲染哪个组件」从两个布局壳里各自手写的
 * Switch/Match 收敛到一处声明式表。两壳仍各管自己的外壳(Rail+ContextBar /
 * 顶栏 Tab),只共用这份 page→组件 映射。新增页面 = 加一行,而非改两个 Switch。
 */
export interface Route {
  page: Page;
  component: Component;
  /** 该页是否需要 Modrinth 布局的右侧上下文栏(ContextBar)。 */
  showContext?: boolean;
}

/** Modrinth(深色三区)布局的页面;首项为未命中时的兜底。 */
export const MODRINTH_ROUTES: Route[] = [
  { page: "home", component: Home, showContext: true },
  { page: "discover", component: Discover },
  { page: "library", component: Library },
  { page: "settings", component: Settings },
];

/** PCL(浅色顶栏 Tab)布局的页面;首项为兜底。 */
export const PCL_ROUTES: Route[] = [
  { page: "launch", component: PclLaunch },
  { page: "discover", component: Discover },
  { page: "settings", component: Settings },
  { page: "more", component: PclMore },
];

/** 取某布局下 `page` 对应的路由;未命中返回首项(兜底)。 */
export function routeFor(routes: Route[], page: Page): Route {
  return routes.find((r) => r.page === page) ?? routes[0];
}
