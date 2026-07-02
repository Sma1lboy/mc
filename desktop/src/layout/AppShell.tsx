import { useEffect, useState } from "react";
import clsx from "clsx";
import { useAppStore, type Page } from "../store";
import { WORKSPACE_ROUTES, routeFor } from "../routes";
import Rail from "./Rail";
import TopBar from "./TopBar";
import ContextBar from "./ContextBar";
import "./AppShell.css";

/**
 * 常驻(keep-alive)标签页:首次访问后保持挂载,切走只隐藏(display:none)不卸载,
 * 切回即时显示且保留页内状态(发现页的搜索结果 / 滚动位置等),避免重挂导致的卡顿与重拉。
 * 实例详情(instance)按实例不同需重挂,故不在此列,仍按需挂载/卸载。
 */
const KEEP_ALIVE: Page[] = ["home", "discover", "library", "agent", "settings"];

/**
 * AppShell —— 工作台视图的三区 CSS Grid 骨架。
 *
 * 布局:
 *   grid-template-columns: 64px 1fr   ← 左 Rail + 其余
 *   grid-template-rows:    48px 1fr   ← 顶 TopBar + body
 *   areas:  "rail topbar"
 *           "rail body"
 *
 * Rail 跨两行,所以 TopBar / body 都从 64px 之后开始,视觉上 Rail 是一整条竖栏。
 *
 * 新 IA 下账号收成右上芯片、好友/动态移出主区,各页主内容均铺满(单列 1fr)。
 * ContextBar 仍按路由的 showContext 显隐(现全为 false 故不渲染),组件暂留备用:
 * 某页若日后重新需要右栏,把它的 showContext 置 true 即可恢复两列。
 */
export default function AppShell(): React.ReactElement {
  const currentPage = useAppStore((s) => s.currentPage);
  const route = routeFor(WORKSPACE_ROUTES, currentPage);
  const showContext = route.showContext ?? false;
  const currentIsKeepAlive = KEEP_ALIVE.includes(currentPage);

  // keep-alive:记录已访问过的常驻页(惰性挂载——首访才挂,之后常驻)。
  const [visited, setVisited] = useState<Set<Page>>(() => new Set());
  useEffect(() => {
    if (KEEP_ALIVE.includes(currentPage) && !visited.has(currentPage)) {
      setVisited((s) => new Set(s).add(currentPage));
    }
  }, [currentPage, visited]);

  const keepAliveRoutes = WORKSPACE_ROUTES.filter(
    (r) => KEEP_ALIVE.includes(r.page) && visited.has(r.page),
  );

  // 非常驻页(实例详情等)的组件:大写变量供 JSX 实例化。
  const ActiveComponent = route.component;

  return (
    <div className="app-shell grid w-screen h-screen text-fg text-[length:var(--fs-base)] overflow-hidden">
      <Rail />
      <TopBar />
      {/* body:有右栏时两列(1fr 340px),无右栏时单列铺满 */}
      <div
        className={clsx("grid min-h-0 min-w-0 [grid-area:body]", {
          "grid-cols-[1fr]": !showContext,
          "grid-cols-[1fr_340px]": showContext,
        })}
      >
        <main className="[grid-row:1] [grid-column:1] w-full h-full min-w-0 min-h-0 overflow-hidden bg-window">
          {/* 常驻标签页:首访后保持挂载,用 display 切显隐,各自独立滚动(wrapper 自带 overflow)。
              页内已自管滚动(h-full overflow-auto)的页在此正好撑满,不会出现双滚动条。 */}
          {keepAliveRoutes.map((r) => {
            const KeptComponent = r.component;
            return (
              <div
                key={r.page}
                className={clsx(
                  "w-full h-full min-w-0 min-h-0 overflow-y-auto overflow-x-hidden",
                  { hidden: currentPage !== r.page },
                )}
              >
                <KeptComponent />
              </div>
            );
          })}
          {/* 非常驻页(实例详情等):按需挂载/卸载。 */}
          {!currentIsKeepAlive && (
            <div className="w-full h-full min-w-0 min-h-0 overflow-y-auto overflow-x-hidden">
              <ActiveComponent />
            </div>
          )}
        </main>
        {/* 右栏按页面显隐。卸载时整列从 grid 消失,主内容自然铺满。 */}
        {showContext && <ContextBar />}
      </div>
    </div>
  );
}
