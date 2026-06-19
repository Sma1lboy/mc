import { Component, For, Switch, Match, type JSX } from "solid-js";
import { currentPage, setCurrentPage, switchLayout, type Page } from "../../store";
import { Icon, type IconName } from "../../components";
import PclLaunch from "../../pages/PclLaunch";
import PclMore from "../../pages/PclMore";
import Discover from "../../pages/Discover";
import Settings from "../../pages/Settings";
import "./PclShell.css";

/**
 * PclShell —— PCL(Plain Craft Launcher)风格外壳:
 *   - 顶部主题色标题栏 + 横向 Tab 导航,每个 Tab 带图标(启动/下载/设置/更多),
 *     白字、选中态白底圆角蓝字,1:1 照抄 PCL CE 顶栏。
 *   - 浅色基调(由 switchLayout 切到 light + 蓝主题)。
 *   - 页内不再有左侧图标栏;启动页自己做左右分栏(账号卡 + 新闻主页)。
 *
 * 与 Modrinth 外壳并存,数据/IPC/页面尽量复用,只换"皮+导航形态"。
 * 见 docs/05-ui-design-pcl.md。
 */

// 顶栏图标已收口到统一 Icon 注册表(components/Icon.tsx),此处只引名。

// 品牌 Logo:白底圆角方块(app icon 风)+ 蓝色等距 Minecraft 方块,在蓝标题栏上醒目。
const PclLogo = (): JSX.Element => (
  <svg viewBox="0 0 28 28" aria-hidden="true">
    <rect x="0.5" y="0.5" width="27" height="27" rx="7" fill="#fff" />
    <path d="M14 6.5 20.5 10v8L14 21.5 7.5 18v-8L14 6.5Z" fill="#eaf2fe" />
    <path d="M14 6.5 20.5 10 14 13.5 7.5 10 14 6.5Z" fill="#4890f5" />
    <path d="M14 13.5v8L7.5 18v-8l6.5 3.5Z" fill="#1370f3" />
    <path d="M14 13.5v8l6.5-3.5v-8L14 13.5Z" fill="#0b5bcb" />
  </svg>
);

// PCL 顶部 Tab → 页面(图标走统一 Icon 注册表,照抄 PCL CE 的图标语义)。
const TABS: { page: Page; label: string; icon: IconName }[] = [
  { page: "launch", label: "启动", icon: "power" },
  { page: "discover", label: "下载", icon: "download" },
  { page: "settings", label: "设置", icon: "gear" },
  { page: "more", label: "更多", icon: "grid" },
];

const PclShell: Component = () => {
  return (
    <div class="pcl-shell">
      {/* 主题色标题栏(可拖拽);左侧留白避开原生交通灯 */}
      <header class="pcl-titlebar" data-tauri-drag-region>
        <div class="pcl-brand" data-tauri-drag-region>
          <span class="pcl-logo-mark"><PclLogo /></span>
          <span class="pcl-logo">PCL</span>
          <span class="pcl-sub">启动器</span>
        </div>

        <nav class="pcl-tabs no-drag">
          <For each={TABS}>
            {(t) => (
              <button
                class="pcl-tab"
                classList={{ active: currentPage() === t.page }}
                onClick={() => setCurrentPage(t.page)}
              >
                <span class="pcl-tab-icon"><Icon name={t.icon} size={24} /></span>
                {t.label}
              </button>
            )}
          </For>
        </nav>

        <div class="pcl-titlebar-right">
          {/* 一键切回 Modrinth 风,方便对比 */}
          <button class="pcl-switch no-drag" onClick={() => switchLayout("modrinth")}>
            切到 Modrinth 风 ⇄
          </button>
        </div>
      </header>

      <main class="pcl-content">
        <Switch fallback={<PclLaunch />}>
          <Match when={currentPage() === "launch"}>
            <PclLaunch />
          </Match>
          <Match when={currentPage() === "discover"}>
            <Discover />
          </Match>
          <Match when={currentPage() === "settings"}>
            <Settings />
          </Match>
          <Match when={currentPage() === "more"}>
            <PclMore />
          </Match>
        </Switch>
      </main>
    </div>
  );
};

export default PclShell;
