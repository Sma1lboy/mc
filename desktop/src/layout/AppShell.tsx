import { Component, Show } from "solid-js";
import { Dynamic } from "solid-js/web";
import { currentPage } from "../store";
import { WORKSPACE_ROUTES, routeFor } from "../routes";
import Rail from "./Rail";
import TopBar from "./TopBar";
import ContextBar from "./ContextBar";
import "./AppShell.css";

/**
 * AppShell —— 工作台视图的三区 CSS Grid 骨架。
 *
 * 布局:
 *   grid-template-columns: 64px 1fr   ← 左 Rail + 其余
 *   grid-template-rows:    48px 1fr   ← 顶 TopBar + body
 *   areas:  "rail topbar"
 *           "rail body"
 *
 * body 内再分 1fr 340px(主内容 + ContextBar)。Rail 跨两行,
 * 所以 TopBar / body 都从 64px 之后开始,视觉上 Rail 是一整条竖栏。
 *
 * ContextBar 是「上下文相关」的:只有 Home 这类需要右栏的页面才渲染它,
 * library / discover / settings 让主内容铺满(此时 body 变成单列 1fr)。
 */

const AppShell: Component = () => {
  // 当前页对应的路由(组件 + 是否需要右栏)。currentPage 是 signal,读它即建立响应依赖。
  const route = () => routeFor(WORKSPACE_ROUTES, currentPage());
  const showContext = () => route().showContext ?? false;

  return (
    <div class="app-shell grid w-screen h-screen bg-n-1 text-fg text-[length:var(--fs-base)] overflow-hidden">
      <Rail />
      <TopBar />
      {/* body:有右栏时两列(1fr 340px),无右栏时单列铺满 */}
      <div
        class="grid min-h-0 min-w-0 [grid-area:body]"
        classList={{
          "grid-cols-[1fr]": !showContext(),
          "grid-cols-[1fr_340px]": showContext(),
        }}
      >
        <main class="[grid-row:1] [grid-column:1] min-w-0 min-h-0 overflow-y-auto overflow-x-hidden bg-n-3">
          {/* 根据 currentPage 从路由表取组件渲染(同一时刻只挂一个页面)。 */}
          <Dynamic component={route().component} />
        </main>
        {/* 右栏按页面显隐。Show 卸载时整列从 grid 消失,主内容自然铺满。 */}
        <Show when={showContext()}>
          <ContextBar />
        </Show>
      </div>
    </div>
  );
};

export default AppShell;
