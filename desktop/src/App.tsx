import { useEffect } from "react";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { initTheme } from "./theme/theme";
import { api } from "./ipc/api";
import { currentRoot, setCurrentRoot } from "./store";
import { useLang } from "./i18n";

/**
 * 应用根组件(阶段①占位骨架)。
 *
 * 真正的三区布局 <AppShell/> 在阶段③接入;这里只做与框架相关的根级接线:
 *   1) 挂载时初始化主题(theme.ts 仍为旧实现,initTheme 公开签名与框架无关,后续阶段再移植)。
 *   2) 选定默认游戏根目录(发现的第一个),供各页作为查询参数。
 *   3) 全局外链拦截:webview 里点 http(s) 链接改用系统浏览器打开。
 *   4) useLang() 订阅当前语言,使整棵(未 memo 的)子树在切语言时重渲染。
 */
export default function App(): React.ReactElement {
  useLang(); // 语言变化 → 重渲染整棵子树(t() 重新取值)

  useEffect(() => {
    void initTheme();

    api
      .listRoots()
      .then((roots) => {
        if (roots.length === 0) return;
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
      if (!/^https?:\/\//i.test(href)) return;
      e.preventDefault();
      void shellOpen(href).catch(() => window.open(href, "_blank"));
    };
    document.addEventListener("click", onDocClick);
    return () => document.removeEventListener("click", onDocClick);
  }, []);

  // 阶段①占位:确认 React + 主题 + i18n 三者接通。阶段③替换为 <AppShell/>。
  // 文案用品牌专名 kobeMC(不翻译),占位屏在阶段③被替换,不引入新词条。
  return (
    <div className="w-screen h-screen flex items-center justify-center text-fg bg-window">
      <p className="text-muted">kobeMC</p>
    </div>
  );
}
