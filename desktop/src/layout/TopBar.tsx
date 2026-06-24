import { Component, For, Show } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { runningIds, currentPage, discoverKind, setDiscoverKind, DISCOVER_KINDS } from "../store";
import { DownloadQueue } from "../components/DownloadQueue";
import { t } from "../i18n";

/**
 * TopBar —— 48px 顶栏(无边框窗口的自绘标题栏)。
 *
 * 左: 空拖拽区(页面标题由各页自己的 H1 承担,顶栏不再重复)。
 * 右: 运行状态药丸 + 品牌名(kobeMC,灰) + 窗口控制(最小化/关闭)。
 *
 * 拖拽:整条 .topbar 用 data-tauri-drag-region(Tauri v2 原生属性)实现窗口拖动;
 * 所有可点击区域(窗口按钮)用 .no-drag(-webkit-app-region:no-drag)排除,
 * 否则点击会被拖拽吞掉。
 */

const MinimizeIcon = () => (
  <svg class="w-[12px] h-[12px]" viewBox="0 0 12 12" aria-hidden="true">
    <line x1="2.5" y1="6" x2="9.5" y2="6" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" />
  </svg>
);

const CloseIcon = () => (
  <svg class="w-[12px] h-[12px]" viewBox="0 0 12 12" aria-hidden="true">
    <line x1="3" y1="3" x2="9" y2="9" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" />
    <line x1="9" y1="3" x2="3" y2="9" stroke="currentColor" stroke-width="1.1" stroke-linecap="round" />
  </svg>
);

// 惰性、带兜底地调用 Tauri 窗口 API:绝不在渲染期调用(否则 __TAURI_INTERNALS__
// 未就绪会读 .metadata 抛错白屏),只在用户点击时执行;非 Tauri(浏览器预览)环境
// 静默忽略。
function windowAction(action: (w: ReturnType<typeof getCurrentWindow>) => void) {
  try {
    action(getCurrentWindow());
  } catch (e) {
    console.warn("窗口 API 不可用(非 Tauri 环境?)", e);
  }
}

const TopBar: Component = () => {
  // 运行状态用 store 的实时集合(game://started/exit 事件驱动),与实例行同源,避免
  // 顶栏自己再拉一份 list_instances 导致「行显示运行中、顶栏却说无实例运行」的不一致。
  const runningCount = () => runningIds().size;

  return (
    // data-tauri-drag-region:让顶栏空白处可拖动窗口
    <header
      class="[grid-area:topbar] h-[48px] flex items-center justify-between glass-panel border-b border-glass-divider pl-[14px] pr-[8px] box-border select-none"
      data-tauri-drag-region
    >
      {/* 左侧:Discover 内容类型标签(仅发现页显示)。标签上提到顶栏,页面下方就纯粹是筛选 + 内容。 */}
      <div class="flex items-center gap-[4px] [-webkit-app-region:no-drag]">
        <Show when={currentPage() === "discover"}>
          <For each={DISCOVER_KINDS}>
            {(k) => (
              <button
                class="h-[28px] px-[12px] rounded-ctl border-none text-[12px] font-medium cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
                classList={{
                  "bg-a-4 text-white": discoverKind() === k,
                  "bg-transparent text-dim hover:text-fg hover:bg-glass-hover": discoverKind() !== k,
                }}
                onClick={() => setDiscoverKind(k)}
              >
                {t(`discover.kind${k[0].toUpperCase()}${k.slice(1)}`)}
              </button>
            )}
          </For>
        </Show>
      </div>

      {/* 右侧:下载队列 + 运行状态(玻璃药丸)+ 品牌名 + 窗口控制 */}
      <div class="flex items-center gap-[10px]">
        <DownloadQueue />
        <div
          class="inline-flex items-center gap-[6px] h-[26px] pl-[9px] pr-[11px] rounded-full bg-glass-card border border-glass-border"
          data-tauri-drag-region
        >
          <Show
            when={runningCount() > 0}
            fallback={
              <>
                <span class="w-[7px] h-[7px] rounded-full shrink-0 bg-n-6" aria-hidden="true" />
                <span class="text-[12px] text-dim whitespace-nowrap">{t("layout.noInstanceRunning")}</span>
              </>
            }
          >
            <span class="w-[7px] h-[7px] rounded-full shrink-0 bg-a-5" aria-hidden="true" />
            <span class="text-[12px] text-fg whitespace-nowrap">{t("layout.running", { n: runningCount() })}</span>
          </Show>
        </div>

        {/* 品牌名:低存在感的灰色字,落在右上角。 */}
        <span
          class="text-[12px] text-dim font-semibold tracking-[0.3px] whitespace-nowrap"
          data-tauri-drag-region
        >
          kobeMC
        </span>

        {/* 窗口控制:no-drag,调 Tauri window API。原生交通灯按钮已提供,这里隐藏自绘控制以免重复。 */}
        <div class="hidden items-center gap-[2px] [-webkit-app-region:no-drag]">
          <button
            class="w-[30px] h-[30px] border-none bg-transparent rounded-ctl text-n-7 cursor-pointer grid place-items-center transition-[background-color,color] duration-[var(--dur)] ease-app motion-reduce:transition-none hover:bg-glass-hover hover:text-fg"
            title={t("layout.minimize")}
            aria-label={t("layout.minimize")}
            onClick={() => windowAction((w) => w.minimize())}
          >
            <MinimizeIcon />
          </button>
          <button
            class="w-[30px] h-[30px] border-none bg-transparent rounded-ctl text-n-7 cursor-pointer grid place-items-center transition-[background-color,color] duration-[var(--dur)] ease-app motion-reduce:transition-none hover:bg-danger hover:text-white"
            title={t("layout.close")}
            aria-label={t("layout.close")}
            onClick={() => windowAction((w) => w.close())}
          >
            <CloseIcon />
          </button>
        </div>
      </div>
    </header>
  );
};

export default TopBar;
