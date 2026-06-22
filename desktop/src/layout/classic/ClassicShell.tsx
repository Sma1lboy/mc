import { Component, For, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { currentPage, setCurrentPage, switchLayout, type Page } from "../../store";
import { Icon, type IconName } from "../../components";
import { CLASSIC_ROUTES, routeFor } from "../../routes";
import "./ClassicShell.css";

/**
 * ClassicShell —— 经典顶栏外壳:
 *   - 顶部主题色标题栏 + 横向 Tab 导航,每个 Tab 带图标(启动/下载/设置/更多)。
 *   - 浅色基调(由 switchLayout 切到 light + 蓝主题)。
 *   - 页内不再有左侧图标栏;启动页自己做左右分栏(账号卡 + 新闻主页)。
 *
 * 与工作台外壳并存,数据/IPC/页面尽量复用,只换视觉层与导航形态。
 */

// 顶栏图标已收口到统一 Icon 注册表(components/Icon.tsx),此处只引名。

// 品牌 Logo:白底圆角方块 + 蓝色等距 Minecraft 方块,在蓝标题栏上醒目。
const ClassicLogo = (): JSX.Element => (
  <svg viewBox="0 0 28 28" aria-hidden="true">
    <rect x="0.5" y="0.5" width="27" height="27" rx="7" fill="#fff" />
    <path d="M14 6.5 20.5 10v8L14 21.5 7.5 18v-8L14 6.5Z" fill="#eaf2fe" />
    <path d="M14 6.5 20.5 10 14 13.5 7.5 10 14 6.5Z" fill="#4890f5" />
    <path d="M14 13.5v8L7.5 18v-8l6.5 3.5Z" fill="#1370f3" />
    <path d="M14 13.5v8l6.5-3.5v-8L14 13.5Z" fill="#0b5bcb" />
  </svg>
);

// 经典顶栏 Tab → 页面(图标走统一 Icon 注册表)。
const TABS: { page: Page; label: string; icon: IconName }[] = [
  { page: "launch", label: "启动", icon: "power" },
  { page: "discover", label: "下载", icon: "download" },
  { page: "settings", label: "设置", icon: "gear" },
  { page: "more", label: "更多", icon: "grid" },
];

const ClassicShell: Component = () => {
  return (
    // classic-shell 仅保留为变量定义 scope(--classic-* 在残留 CSS 里),布局已迁到工具类。
    <div class="classic-shell grid grid-rows-[48px_1fr] w-screen h-screen overflow-hidden bg-classic-gray-bg text-classic-text font-[Microsoft_YaHei_UI,'PingFang_SC',system-ui,sans-serif] text-[13px]">
      {/* 主题色标题栏(可拖拽);左侧留白避开原生交通灯 */}
      <header
        class="flex items-center gap-[18px] pl-[80px] pr-[12px] bg-[linear-gradient(90deg,var(--classic-blue-hover),var(--classic-blue))] text-white select-none shadow-[0_1px_6px_rgba(52,61,74,0.18)] z-[1]"
        data-tauri-drag-region
      >
        <div class="flex items-center gap-[9px]" data-tauri-drag-region>
          <span class="flex w-[28px] h-[28px] flex-[0_0_28px] [filter:drop-shadow(0_1px_3px_rgba(11,91,203,0.35))] [&_svg]:w-full [&_svg]:h-full"><ClassicLogo /></span>
          <span class="text-[19px] font-extrabold tracking-[1px] self-center" translate="no">MC Launcher</span>
        </div>

        <nav class="flex gap-[2px] h-full items-center [-webkit-app-region:no-drag]">
          <For each={TABS}>
            {(t) => (
              <button
                class="flex items-center gap-[6px] h-[32px] px-[15px] border-none bg-transparent text-white text-[14px] rounded-[4px] cursor-pointer transition-[background] duration-[0.18s] ease-[ease] hover:bg-white/20"
                classList={{
                  "!bg-white !text-classic-blue !font-semibold": currentPage() === t.page,
                }}
                onClick={() => setCurrentPage(t.page)}
              >
                <span class="flex w-[17px] h-[17px] [&_svg]:w-full [&_svg]:h-full"><Icon name={t.icon} size={24} /></span>
                {t.label}
              </button>
            )}
          </For>
        </nav>

        <div class="ml-auto flex items-center">
          {/* 一键切回工作台视图,方便对比 */}
          <button
            class="border border-white/55 bg-white/[0.14] text-white text-[12px] px-[11px] py-[5px] rounded-[3px] cursor-pointer transition-[background] duration-[0.18s] ease-[ease] hover:bg-white/30 [-webkit-app-region:no-drag]"
            onClick={() => switchLayout("workspace")}
          >
            切到工作台视图 ⇄
          </button>
        </div>
      </header>

      <main class="w-full h-full overflow-hidden min-h-0 bg-classic-gray-bg">
        {/* 从路由表取当前页组件渲染。 */}
        <Dynamic component={routeFor(CLASSIC_ROUTES, currentPage()).component} />
      </main>
    </div>
  );
};

export default ClassicShell;
