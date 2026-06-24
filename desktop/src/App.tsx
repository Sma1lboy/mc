import { onCleanup, onMount, type Component } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { initTheme } from "./theme/theme";
import AppShell from "./layout/AppShell";
import { ToastContainer } from "./components";
import { api } from "./ipc/api";
import { currentRoot, setCurrentRoot } from "./store";
import { maybeRunGallery } from "./gallery/runner";

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
    void initTheme();
    // 选定默认游戏根目录(发现的第一个),供各页面作为查询参数。
    api
      .listRoots()
      .then((roots) => {
        if (roots.length === 0) return;
        // 保留上次选中的根(若仍存在),否则落到发现的第一个。
        const saved = currentRoot();
        const keep = saved && roots.some((r) => r.path === saved);
        if (!keep) setCurrentRoot(roots[0].path);
      })
      .catch(() => {
        /* Tauri 后端不可用时忽略,页面会用 "" 落到后端默认根 */
      });

    // 全局外链拦截:webview 里点 http(s) 链接(含 markdown innerHTML 渲染的链接)若直接导航,
    // 整个 SPA 会被外站替换且无法返回。这里统一拦下,改用系统浏览器打开。一处兜住所有 <a>。
    const onDocClick = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey)
        return;
      const a = (e.target as HTMLElement | null)?.closest("a");
      const href = a?.getAttribute("href") ?? "";
      if (!/^https?:\/\//i.test(href)) return; // 仅拦外部 http(s),站内/锚点照常
      e.preventDefault();
      void shellOpen(href).catch(() => window.open(href, "_blank"));
    };
    document.addEventListener("click", onDocClick);
    onCleanup(() => document.removeEventListener("click", onDocClick));

    // 画廊模式(MC_GALLERY=1):挂载后自动逐页截图并生成 index.html。非画廊模式零开销。
    void maybeRunGallery();
  });

  return (
    <>
      <AppShell />
      {/* 全局 Toast 容器:左下角滑入提示,挂在根部一次即可。 */}
      <ToastContainer />
    </>
  );
};

export default App;
