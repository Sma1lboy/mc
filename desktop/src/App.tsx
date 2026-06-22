import { onMount, Show, type Component } from "solid-js";
import { initTheme, applyTheme, themeForLayout } from "./theme/theme";
import AppShell from "./layout/AppShell";
import ClassicShell from "./layout/classic/ClassicShell";
import { ToastContainer } from "./components";
import { api } from "./ipc/api";
import { setCurrentRoot, layoutMode } from "./store";

/**
 * 应用根组件。
 *
 * 职责:
 *   1) 挂载时初始化主题(向后端取 get_theme,失败回落默认深色绿),
 *      在首帧后立即把 accent 色阶与 data-theme 注入到 html 根元素。
 *   2) 渲染三区布局骨架 <AppShell/>。
 *
 * 页面路由不走重型 Router:用 store.ts 暴露的 currentPage 信号即可,
 * AppShell 内部据此分发到具体页面。状态在模块作用域,组件直接 import 读写。
 */
const App: Component = () => {
  onMount(() => {
    // 异步初始化主题,不阻塞渲染;tokens.css 已提供默认值兜底首帧观感。
    // 启动后让主题与当前布局相称:经典布局 → 浅色+蓝。
    initTheme().then(() => {
      // 经典布局启动时套用其相称主题(浅色+蓝);经 themeForLayout 这一处获取
      // 每布局默认,与 switchLayout / 设置页重置同源,不再散落硬编码常量。
      if (layoutMode() === "classic") {
        applyTheme(themeForLayout(layoutMode()));
      }
    });
    // 选定默认游戏根目录(发现的第一个),供各页面作为查询参数。
    api
      .listRoots()
      .then((roots) => {
        if (roots.length > 0) setCurrentRoot(roots[0].path);
      })
      .catch(() => {
        /* Tauri 后端不可用时忽略,页面会用 "" 落到后端默认根 */
      });
  });

  return (
    <>
      {/* 两套界面:工作台视图与经典视图,按 layoutMode 切换。 */}
      <Show when={layoutMode() === "classic"} fallback={<AppShell />}>
        <ClassicShell />
      </Show>
      {/* 全局 Toast 容器:左下角滑入提示,挂在根部一次即可。 */}
      <ToastContainer />
    </>
  );
};

export default App;
