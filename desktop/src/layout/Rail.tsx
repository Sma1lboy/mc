import { Component, JSX, For, createSignal } from "solid-js";
import {
  AccountMenu,
  BlockIcon,
  InstanceIcon,
  NavItem,
  NewInstanceDialog,
} from "../components";
import { currentPage, setCurrentPage, openInstance, currentInstanceId, instances, refreshInstances } from "../store";
import { sortByRecent } from "../util/instances";
import { t } from "../i18n";
import "./Rail.css";

/**
 * Rail —— 64px 石质竖栏(Blocky Craft 主导航形态)。
 *
 * 自上而下:草方块 Logo → 主导航(首页 / 发现 / 库)→ 2px 暗分隔 →
 * 最近实例快捷方块 → 虚线「+新建」→ mt-auto 撑开 → 设置 → 账号头像方块。
 * 选中态走 NavItem 的熔岩橙凸起;实例快捷块用凸起倒角的 InstanceIcon。
 */

// 主导航项。id 必须与 store 的 Page 联合类型一致(home/discover/library/settings)。
type NavId = "home" | "discover" | "library" | "settings";

interface NavEntry {
  id: NavId;
  label: string;
  icon: () => JSX.Element;
}

// 内联 SVG 图标:线性、24x24、stroke 2.2、fill=none stroke=currentColor。
const HomeIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
       stroke-linecap="round" stroke-linejoin="round" class="w-[22px] h-[22px]" aria-hidden="true">
    <path d="M3 10.5 12 3l9 7.5" />
    <path d="M5 9.5V20a1 1 0 0 0 1 1h3v-6h6v6h3a1 1 0 0 0 1-1V9.5" />
  </svg>
);

// 发现:罗盘(compass)。
const DiscoverIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
       stroke-linecap="round" stroke-linejoin="round" class="w-[22px] h-[22px]" aria-hidden="true">
    <circle cx="12" cy="12" r="9" />
    <path d="m15.5 8.5-2.2 4.8-4.8 2.2 2.2-4.8 4.8-2.2Z" />
  </svg>
);

// 库:书本(book)。
const LibraryIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
       stroke-linecap="round" stroke-linejoin="round" class="w-[22px] h-[22px]" aria-hidden="true">
    <path d="M4 5.5A2 2 0 0 1 6 4h13v15H6a2 2 0 0 0-2 2V5.5Z" />
    <path d="M4 19.5A2 2 0 0 1 6 18h13" />
  </svg>
);

// 设置:滑块(sliders)。
const SettingsIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
       stroke-linecap="round" stroke-linejoin="round" class="w-[22px] h-[22px]" aria-hidden="true">
    <path d="M4 7h10M18 7h2M4 17h2M10 17h10" />
    <circle cx="16" cy="7" r="2" />
    <circle cx="8" cy="17" r="2" />
  </svg>
);

const PlusIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2"
       stroke-linecap="round" stroke-linejoin="round" class="w-[18px] h-[18px]" aria-hidden="true">
    <path d="M12 5v14M5 12h14" />
  </svg>
);

const TOP_NAV = (): NavEntry[] => [
  { id: "home", label: t("layout.navHome"), icon: HomeIcon },
  { id: "discover", label: t("layout.navDiscover"), icon: DiscoverIcon },
  { id: "library", label: t("layout.navLibrary"), icon: LibraryIcon },
];

const Rail: Component = () => {
  // 最近实例:按上次游玩排序取前 3,作为 rail 快捷入口(点击进详情)。
  // 实例列表来自全局 store(单一真相),库页删除后这里同步更新。
  const pinned = () => sortByRecent(instances() ?? []).slice(0, 3);

  // 新增实例对话框。
  const [newOpen, setNewOpen] = createSignal(false);

  return (
    <nav
      class="stone [grid-area:rail] w-[64px] h-full flex flex-col items-center border-r-2 border-titlebar pt-[34px] pb-[10px] gap-[6px] box-border"
      aria-label={t("layout.primaryNav")}
    >
      {/* 顶部 Logo:草方块 */}
      <button
        type="button"
        class="border-none bg-transparent cursor-pointer p-0 mb-[2px] transition-transform duration-[var(--dur)] ease-app active:scale-[0.94] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
        title="kobeMC"
        onClick={() => setCurrentPage("home")}
      >
        <BlockIcon class="w-[32px] h-[32px]" />
      </button>

      {/* 主导航 */}
      <div class="flex flex-col items-center gap-[4px]">
        <For each={TOP_NAV()}>
          {(item) => {
            const selected = () => currentPage() === item.id;
            return (
              <NavItem
                active={selected()}
                title={item.label}
                onClick={() => setCurrentPage(item.id)}
              >
                {item.icon()}
              </NavItem>
            );
          }}
        </For>
      </div>

      {/* 2px 暗分隔 */}
      <div class="w-[34px] h-[2px] bg-titlebar my-[4px] shrink-0" aria-hidden="true" />

      {/* 中部:最近实例快捷方块 + 新增(可滚动) */}
      <div
        class="rail-pinned flex-[1_1_auto] w-full flex flex-col items-center gap-[6px] py-[4px] overflow-y-auto overflow-x-hidden min-h-0"
        aria-label={t("layout.recentInstances")}
      >
        <For each={pinned()}>
          {(inst) => {
            const selected = () => currentPage() === "instance" && currentInstanceId() === inst.id;
            return (
              <button
                type="button"
                class={`w-[36px] h-[36px] shrink-0 border-none bg-transparent p-0 cursor-pointer shadow-raised transition-transform duration-[var(--dur)] ease-app active:scale-[0.94] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent${
                  selected() ? " ring-2 ring-accent" : ""
                }`}
                title={inst.name || inst.id}
                onClick={() => openInstance(inst.id)}
              >
                <span class="block w-full h-full overflow-hidden select-none">
                  <InstanceIcon name={inst.name || inst.id} icon={inst.icon ?? undefined} />
                </span>
              </button>
            );
          }}
        </For>

        {/* 新增实例:虚线 + 方块,点开新建对话框。 */}
        <button
          type="button"
          class="w-[36px] h-[36px] shrink-0 grid place-items-center border-2 border-dashed border-titlebar bg-transparent text-muted cursor-pointer transition-[color,border-color] duration-[var(--dur)] ease-app hover:text-accent hover:border-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          title={t("layout.newInstance")}
          onClick={() => setNewOpen(true)}
        >
          <PlusIcon />
        </button>
      </div>

      {/* mt-auto 撑开 → 设置 + 账号头像。 */}
      <div class="mt-auto flex flex-col items-center gap-[8px] shrink-0">
        {(() => {
          const selected = () => currentPage() === "settings";
          return (
            <NavItem
              active={selected()}
              title={t("layout.navSettings")}
              onClick={() => setCurrentPage("settings")}
            >
              <SettingsIcon />
            </NavItem>
          );
        })()}

        {/* 账号:头像方块,点开切换/添加账号(AccountMenu 自带列表 + 登录弹窗)。 */}
        <AccountMenu variant="avatar" />
      </div>

      <NewInstanceDialog
        open={newOpen()}
        onClose={() => setNewOpen(false)}
        onCreated={(id) => {
          void refreshInstances();
          openInstance(id);
        }}
      />
    </nav>
  );
};

export default Rail;
