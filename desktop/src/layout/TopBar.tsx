import { Component, Show } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { runningIds, socialEnabled } from "../store";
import { DownloadQueue } from "../components/DownloadQueue";
import { KobeAccountChip } from "../components/KobeAccountChip";
import { PixelLabel } from "../components";
import { t } from "../i18n";

/**
 * TopBar —— 48px 顶栏(无边框窗口的自绘标题栏,Blocky Craft 石质标题栏)。
 *
 * 左: 空拖拽区(页面标题由各页 H1 承担;Discover 的内容类型 tab 落在发现页内)。
 * 右: 下载队列 + 运行状态(凹陷方块药丸,状态点 + 文案)+ 品牌名 kobeMC(点阵沙金)。
 *
 * 拖拽:整条顶栏 data-tauri-drag-region 实现窗口拖动;可点区域用 -webkit-app-region:no-drag
 * 排除,否则点击会被拖拽吞掉。
 */

const MinimizeIcon = () => (
  <svg class="w-[12px] h-[12px]" viewBox="0 0 12 12" aria-hidden="true">
    <line x1="2.5" y1="6" x2="9.5" y2="6" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" />
  </svg>
);

const CloseIcon = () => (
  <svg class="w-[12px] h-[12px]" viewBox="0 0 12 12" aria-hidden="true">
    <line x1="3" y1="3" x2="9" y2="9" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" />
    <line x1="9" y1="3" x2="3" y2="9" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" />
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
      class="[grid-area:topbar] h-[48px] flex items-center justify-between bg-titlebar border-b border-titlebar pl-[12px] pr-[8px] box-border select-none"
      data-tauri-drag-region
    >
      {/* 左侧:空拖拽区(占位,把右侧控件推到右上角)。 */}
      <div class="flex-1 h-full" data-tauri-drag-region />

      {/* 右侧:下载队列 + 运行状态(凹陷方块药丸)+ 品牌名 + 窗口控制 */}
      <div class="flex items-center gap-[10px]">
        <DownloadQueue />

        {/* 运行状态:凹陷方块药丸,直角倒角。 */}
        <div
          class="inline-flex items-center gap-[7px] h-[26px] px-[10px] bg-panel-2 shadow-sunken"
          data-tauri-drag-region
        >
          <Show
            when={runningCount() > 0}
            fallback={
              <>
                <span class="w-[7px] h-[7px] shrink-0 bg-muted" aria-hidden="true" />
                <span class="text-[12px] text-muted whitespace-nowrap">{t("layout.noInstanceRunning")}</span>
              </>
            }
          >
            <span class="w-[7px] h-[7px] shrink-0 bg-accent" aria-hidden="true" />
            <span class="text-[12px] text-fg whitespace-nowrap">{t("layout.running", { n: runningCount() })}</span>
          </Show>
        </div>

        {/* kobeMC 账号入口(社交开关关闭时整体隐藏)。 */}
        <Show when={socialEnabled()}>
          <KobeAccountChip />
        </Show>

        {/* 品牌名:点阵沙金短词,落在右上角。 */}
        <PixelLabel
          class="text-[11px] text-tag tracking-[0.5px] whitespace-nowrap"
          data-tauri-drag-region
        >
          kobeMC
        </PixelLabel>

        {/* 窗口控制:no-drag,调 Tauri window API。原生交通灯按钮已提供,这里隐藏自绘控制以免重复。 */}
        <div class="hidden items-center gap-[2px] [-webkit-app-region:no-drag]">
          <button
            class="w-[30px] h-[30px] border-none bg-panel-3 text-sub cursor-pointer grid place-items-center shadow-raised active:shadow-pressed transition-colors duration-[var(--dur)] ease-app motion-reduce:transition-none hover:text-fg"
            title={t("layout.minimize")}
            aria-label={t("layout.minimize")}
            onClick={() => windowAction((w) => w.minimize())}
          >
            <MinimizeIcon />
          </button>
          <button
            class="w-[30px] h-[30px] border-none bg-panel-3 text-sub cursor-pointer grid place-items-center shadow-raised active:shadow-pressed transition-colors duration-[var(--dur)] ease-app motion-reduce:transition-none hover:bg-danger hover:text-danger-text"
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
