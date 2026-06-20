import { Component, JSX, For, Show, createResource } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { Avatar } from "../components";
import { currentPage, setCurrentPage } from "../store";
import type { AccountSummary } from "../ipc/types";
import "./Rail.css";

/**
 * Rail —— 64px 竖直图标栏(Modrinth 主导航形态)。
 *
 * 结构(自上而下):
 *   - App Logo
 *   - 主导航:home / discover / library(内联 SVG 图标)
 *   - 分隔线
 *   - 中部:固定实例占位(未来可拖入置顶实例)
 *   - 底部:settings 图标 + 账号头像
 *
 * 选中态:左侧 4px accent 竖条 + 图标变 accent 色;
 * Hover:半透明 accent 底。背景 --n-2。
 */

// 主导航项。id 必须与 store 的 Page 联合类型一致(home/discover/library/settings)。
type NavId = "home" | "discover" | "library" | "settings";

interface NavItem {
  id: NavId;
  label: string;
  icon: () => JSX.Element;
}

// 内联 SVG 图标(线性、24x24、currentColor 描边),与 PCL 的 path 风格统一。
const HomeIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="M3 10.5 12 3l9 7.5" />
    <path d="M5 9.5V20a1 1 0 0 0 1 1h3v-6h6v6h3a1 1 0 0 0 1-1V9.5" />
  </svg>
);

const DiscoverIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="9" />
    <path d="m15.5 8.5-2 5-5 2 2-5 5-2Z" />
  </svg>
);

const LibraryIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <rect x="3" y="4" width="6" height="16" rx="1" />
    <rect x="11" y="4" width="6" height="16" rx="1" />
    <path d="M19.5 5.2 21 19" />
  </svg>
);

const SettingsIcon = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 13a7.8 7.8 0 0 0 0-2l1.8-1.4-1.9-3.3-2.2.9a7.6 7.6 0 0 0-1.7-1l-.3-2.3H10.5l-.3 2.3a7.6 7.6 0 0 0-1.7 1l-2.2-.9-1.9 3.3L6.2 11a7.8 7.8 0 0 0 0 2l-1.8 1.4 1.9 3.3 2.2-.9a7.6 7.6 0 0 0 1.7 1l.3 2.3h3.9l.3-2.3a7.6 7.6 0 0 0 1.7-1l2.2.9 1.9-3.3Z" />
  </svg>
);

// App Logo:简单的方块「方块世界」标记,accent 填充。
const LogoMark = (): JSX.Element => (
  <svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <path d="M12 2 3 7v10l9 5 9-5V7l-9-5Z" fill="var(--a-1)" stroke="var(--a-4)" stroke-width="1.4" stroke-linejoin="round" />
    <path d="M12 7 7.5 9.5v5L12 17l4.5-2.5v-5L12 7Z" fill="var(--a-4)" />
  </svg>
);

const TOP_NAV: NavItem[] = [
  { id: "home", label: "主页", icon: HomeIcon },
  { id: "discover", label: "发现", icon: DiscoverIcon },
  { id: "library", label: "库", icon: LibraryIcon },
];

// 主导航 / 设置按钮通用类。relative + 48x48 + 居中网格 + accent hover/选中态。
// 默认中性图标色 --n-7;hover 半透明 accent 底 + --n-8;选中 accent 色 + 更淡的底。
const RAIL_ITEM =
  "relative w-[48px] h-[48px] border-0 bg-transparent rounded-ctl cursor-pointer " +
  "grid place-items-center text-n-7 " +
  "transition-[background-color,color] duration-200 ease-app motion-reduce:transition-none " +
  "hover:bg-[color-mix(in_srgb,var(--a-4)_16%,transparent)] hover:text-n-8";
// 选中态附加类(配合 RAIL_ITEM 使用)。
const RAIL_ITEM_SELECTED =
  "text-a-5 bg-[color-mix(in_srgb,var(--a-4)_12%,transparent)]";
// 左侧 4px accent 竖条:默认 scaleY(0)+透明,选中时展开。
const RAIL_BAR =
  "absolute left-[-8px] top-1/2 w-[4px] h-[24px] rounded-[0_2px_2px_0] bg-a-4 " +
  "-translate-y-1/2 scale-y-0 opacity-0 origin-center " +
  "transition-[transform,opacity] duration-200 ease-app motion-reduce:transition-none";
// 选中时竖条展开类(覆盖 RAIL_BAR 的隐藏变换)。
const RAIL_BAR_SELECTED = "scale-y-100 opacity-100";
// 图标容器:居中网格,内部 svg 固定 22x22。
const RAIL_ICON = "grid place-items-center [&_svg]:w-[22px] [&_svg]:h-[22px]";

const Rail: Component = () => {
  // 拉取账号用于底部头像。失败/空态都要稳:取 selected 账号,否则取第一个。
  const [accounts] = createResource<AccountSummary[]>(async () => {
    return await invoke<AccountSummary[]>("list_accounts");
  });

  // 当前账号(用于头像首字母)。无数据时返回 undefined,渲染占位。
  const activeAccount = (): AccountSummary | undefined => {
    const list = accounts();
    if (!list || list.length === 0) return undefined;
    return list.find((a) => a.selected) ?? list[0];
  };

  return (
    <nav
      class="[grid-area:rail] w-[64px] h-full flex flex-col items-center bg-n-2 border-r border-n-6 pt-[36px] px-0 pb-[8px] gap-[4px] box-border"
      aria-label="主导航"
    >
      {/* 顶部 Logo */}
      <button
        class="w-[40px] h-[40px] mb-[6px] p-[6px] border-0 bg-transparent rounded-ctl cursor-pointer grid place-items-center transition-[background-color,transform] duration-200 ease-app motion-reduce:transition-none hover:bg-[color-mix(in_srgb,var(--a-4)_14%,transparent)] active:scale-[0.94] [&_svg]:w-[26px] [&_svg]:h-[26px]"
        title="MC Launcher"
        onClick={() => setCurrentPage("home")}
      >
        <LogoMark />
      </button>

      {/* 主导航图标 */}
      <div class="flex flex-col items-center gap-[4px]">
        <For each={TOP_NAV}>
          {(item) => {
            const selected = () => currentPage() === item.id;
            return (
              <button
                class={`${RAIL_ITEM}${selected() ? " " + RAIL_ITEM_SELECTED : ""}`}
                title={item.label}
                aria-current={selected() ? "page" : undefined}
                onClick={() => setCurrentPage(item.id)}
              >
                <span
                  class={`${RAIL_BAR}${selected() ? " " + RAIL_BAR_SELECTED : ""}`}
                  aria-hidden="true"
                />
                <span class={RAIL_ICON}>{item.icon()}</span>
              </button>
            );
          }}
        </For>
      </div>

      <div
        class="w-[32px] h-[1px] bg-n-6 my-[6px] shrink-0"
        aria-hidden="true"
      />

      {/* 中部:固定实例占位区(未来拖入置顶实例,可滚动) */}
      <div
        class="rail-pinned flex-[1_1_auto] w-full flex flex-col items-center gap-[4px] overflow-y-auto overflow-x-hidden min-h-0"
        aria-label="固定实例"
      >
        {/* 暂无固定实例:留空。此区随产品演进填充实例图标按钮。 */}
      </div>

      {/* 底部:设置 + 账号头像。flex 把它推到底。 */}
      <div class="flex flex-col items-center gap-[6px] shrink-0 mt-[4px]">
        {(() => {
          const selected = () => currentPage() === "settings";
          return (
            <button
              class={`${RAIL_ITEM}${selected() ? " " + RAIL_ITEM_SELECTED : ""}`}
              title="设置"
              aria-current={selected() ? "page" : undefined}
              onClick={() => setCurrentPage("settings")}
            >
              <span
                class={`${RAIL_BAR}${selected() ? " " + RAIL_BAR_SELECTED : ""}`}
                aria-hidden="true"
              />
              <span class={RAIL_ICON}>
                <SettingsIcon />
              </span>
            </button>
          );
        })()}

        {/* 账号头像:点击进设置(账号管理也在设置/右栏)。加载中显示占位环。 */}
        <button
          class="w-[36px] h-[36px] border border-n-6 bg-n-4 rounded-full cursor-pointer grid place-items-center overflow-hidden transition-[border-color,transform] duration-200 ease-app motion-reduce:transition-none hover:border-a-4 active:scale-[0.94]"
          title={activeAccount()?.username ?? "账号"}
          onClick={() => setCurrentPage("settings")}
        >
          <Show
            when={!accounts.loading}
            fallback={
              <span
                class="rail-pulse-anim w-full h-full bg-n-5"
                aria-hidden="true"
              />
            }
          >
            <Avatar
              class="rail-avatar-img"
              kind={activeAccount()?.kind}
              uuid={activeAccount()?.uuid}
            />
          </Show>
        </button>
      </div>
    </nav>
  );
};

export default Rail;
