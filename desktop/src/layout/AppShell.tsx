import { Component, Show, Switch, Match } from "solid-js";
import { currentPage } from "../store";
import Home from "../pages/Home";
import Library from "../pages/Library";
import Discover from "../pages/Discover";
import Settings from "../pages/Settings";
import Rail from "./Rail";
import TopBar from "./TopBar";
import ContextBar from "./ContextBar";
import "./AppShell.css";

/**
 * AppShell —— 三区 CSS Grid 骨架(对标 Modrinth,皮用 PCL token)。
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

// 哪些页面需要右侧 ContextBar。Home 显示账号/好友/新闻;其余页面右栏让位给主内容。
const PAGES_WITH_CONTEXT = new Set(["home"]);

const AppShell: Component = () => {
  // 是否渲染右栏。currentPage 是 store 里的 signal accessor,这里读它即建立响应依赖。
  const showContext = () => PAGES_WITH_CONTEXT.has(currentPage());

  return (
    <div class="app-shell">
      <Rail />
      <TopBar />
      {/* body:有右栏时两列(1fr 340px),无右栏时单列铺满 */}
      <div class="app-body" classList={{ "with-context": showContext() }}>
        <main class="app-main">
          {/* 根据 currentPage 切换页面组件。Switch/Match 保证同一时刻只挂一个页面。 */}
          <Switch fallback={<Home />}>
            <Match when={currentPage() === "home"}>
              <Home />
            </Match>
            <Match when={currentPage() === "discover"}>
              <Discover />
            </Match>
            <Match when={currentPage() === "library"}>
              <Library />
            </Match>
            <Match when={currentPage() === "settings"}>
              <Settings />
            </Match>
          </Switch>
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
