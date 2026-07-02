import { useEffect } from "react";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { initTheme } from "./theme/theme";
import AppShell from "./layout/AppShell";
import { ToastContainer, ShortcutsHelp, CrashDialog } from "./components";
import { api } from "./ipc/api";
import { currentRoot, setCurrentRoot } from "./store";
import { registerGlobalShortcuts } from "./util/shortcuts";
import { maybeRunGallery } from "./gallery/runner";
import { useLang } from "./i18n";

/**
 * 应用根组件。
 *
 * 职责(挂载时的根级接线):
 *   1) 初始化主题(theme.ts 仍为旧实现,initTheme 公开签名与框架无关,后续阶段再移植)。
 *   2) 选定默认游戏根目录(发现的第一个),供各页作为查询参数。
 *   3) 全局外链拦截:webview 里点 http(s) 链接改用系统浏览器打开。
 *   4) 全局键盘快捷键(导航 / 快速启动 / 帮助浮层)。
 *   5) 画廊模式(MC_GALLERY=1):挂载后自动逐页截图并生成 index.html。
 *   6) useLang() 订阅当前语言,使整棵(未 memo 的)子树在切语言时重渲染。
 *
 * 页面路由不走重型 Router:用 store 的 currentPage 即可,AppShell 内部据此分发到具体页面。
 */
export default function App(): React.ReactElement {
  useLang(); // 语言变化 → 重渲染整棵子树(t() 重新取值)

  useEffect(() => {
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

    // 全局外链拦截:统一拦下 http(s) <a>,改用系统浏览器打开,避免整个 SPA 被外站替换。
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

    // 全局键盘快捷键(导航 / 快速启动 / 帮助浮层)。一处注册,卸载时解除。
    const unregisterShortcuts = registerGlobalShortcuts();

    // 画廊模式(MC_GALLERY=1):挂载后自动逐页截图并生成 index.html。非画廊模式零开销。
    void maybeRunGallery();

    return () => {
      document.removeEventListener("click", onDocClick);
      unregisterShortcuts();
    };
  }, []);

  return (
    <>
      <AppShell />
      {/* 全局 Toast 容器:左下角滑入提示,挂在根部一次即可。 */}
      <ToastContainer />
      {/* 键盘快捷键速查浮层:由 `?` 切换,挂根部一次即可。 */}
      <ShortcutsHelp />
      {/* 崩溃报告弹窗:游戏异常退出时由 store.crashReport 驱动弹出,挂根部一次即可。 */}
      <CrashDialog />
    </>
  );
}
