import type { Component } from "solid-js";
import type { Page } from "./store";
import Home from "./pages/Home";
import Library from "./pages/Library";
import Discover from "./pages/Discover";
import Settings from "./pages/Settings";
import InstanceDetail from "./pages/InstanceDetail";

/**
 * 页面路由表 —— 把「哪个 page 渲染哪个组件」从布局壳的手写 Switch/Match
 * 收敛到一处声明式表。外壳(Rail + ContextBar)只共用这份 page→组件 映射。
 * 新增页面 = 加一行,而非改 Switch。
 */
export interface Route {
  page: Page;
  component: Component;
  /** 该页是否需要右侧上下文栏(ContextBar)。 */
  showContext?: boolean;
}

/** 全部页面;首项为未命中时的兜底。 */
export const WORKSPACE_ROUTES: Route[] = [
  { page: "home", component: Home, showContext: true },
  { page: "discover", component: Discover },
  { page: "library", component: Library },
  { page: "settings", component: Settings },
  { page: "instance", component: InstanceDetail },
];

/** 取 `page` 对应的路由;未命中返回首项(兜底)。 */
export function routeFor(routes: Route[], page: Page): Route {
  return routes.find((r) => r.page === page) ?? routes[0];
}
