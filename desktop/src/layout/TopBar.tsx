import { Component, Show, createResource, createMemo, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { currentPage, currentRoot } from "../store";
import type { InstanceSummary } from "../ipc/types";

/**
 * TopBar —— 48px 顶栏(无边框窗口的自绘标题栏)。
 *
 * 左: Logo+名(.drag 拖拽区) + 后退/前进箭头(当前无路由历史,占位禁用态) + 当前页标题。
 * 右: 运行状态文字 + 窗口控制(最小化/关闭)。
 *
 * 拖拽:整条 .topbar 用 data-tauri-drag-region(Tauri v2 原生属性)实现窗口拖动;
 * 所有可点击区域(箭头/窗口按钮)用 .no-drag(-webkit-app-region:no-drag)排除,
 * 否则点击会被拖拽吞掉。
 */

// 页面 → 标题文案。与 store 的 Page 联合类型对齐。
const PAGE_TITLES: Record<string, string> = {
  home: "主页",
  discover: "发现",
  library: "库",
  settings: "设置",
  instance: "实例",
};

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

const ArrowLeft = () => (
  <svg class="w-[18px] h-[18px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m14 6-6 6 6 6" />
  </svg>
);

const ArrowRight = () => (
  <svg class="w-[18px] h-[18px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m10 6 6 6-6 6" />
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

  // 运行状态:统计当前 root 下处于 running 的实例数。
  // currentRoot 是 store 的 signal accessor;为空时不查询(返回空列表)。
  const [instances, { refetch }] = createResource<InstanceSummary[], string>(
    () => currentRoot(),
    async (root) => {
      if (!root) return [];
      return await invoke<InstanceSummary[]>("list_instances", { root });
    }
  );

  // 监听启动进度事件:有实例启动/退出时刷新运行状态。
  // launch://progress 在启动链路推送,收到即重新拉取实例列表以更新 running 计数。
  let unlisten: UnlistenFn | undefined;
  onMount(async () => {
    unlisten = await listen("launch://progress", () => {
      // 启动有进展 → 运行状态可能变化,重查。
      refetch();
    });
  });
  onCleanup(() => unlisten?.());

  const runningCount = createMemo(() => {
    const list = instances();
    if (!list) return 0;
    return list.filter((i) => i.running).length;
  });

  const title = () => PAGE_TITLES[currentPage()] ?? "MC Launcher";

  return (
    // data-tauri-drag-region:让顶栏空白处可拖动窗口
    <header
      class="[grid-area:topbar] h-[48px] flex items-center justify-between glass-panel border-b border-glass-divider pl-[14px] pr-[8px] box-border select-none"
      data-tauri-drag-region
    >
      {/* 左侧:Logo 名 + 导航箭头 + 标题 */}
      <div class="flex items-center gap-[10px] min-w-0" data-tauri-drag-region>
        <span
          class="text-[13px] font-semibold text-fg tracking-[0.2px] whitespace-nowrap"
          data-tauri-drag-region
        >
          MC Launcher
        </span>

        {/* 后退/前进:当前无页面历史栈,渲染为禁用占位(no-drag 避免吞点击) */}
        <div class="flex items-center gap-[2px] [-webkit-app-region:no-drag]">
          <button
            class="w-[28px] h-[28px] border-none bg-transparent rounded-ctl text-n-7 cursor-pointer grid place-items-center transition-[background-color,color] duration-[var(--dur)] ease-app motion-reduce:transition-none not-disabled:hover:bg-glass-hover not-disabled:hover:text-fg disabled:opacity-35 disabled:cursor-default"
            disabled
            title="后退"
            aria-label="后退"
          >
            <ArrowLeft />
          </button>
          <button
            class="w-[28px] h-[28px] border-none bg-transparent rounded-ctl text-n-7 cursor-pointer grid place-items-center transition-[background-color,color] duration-[var(--dur)] ease-app motion-reduce:transition-none not-disabled:hover:bg-glass-hover not-disabled:hover:text-fg disabled:opacity-35 disabled:cursor-default"
            disabled
            title="前进"
            aria-label="前进"
          >
            <ArrowRight />
          </button>
        </div>

        <span
          class="text-[var(--fs-base)] font-medium text-fg whitespace-nowrap overflow-hidden text-ellipsis"
          data-tauri-drag-region
        >
          {title()}
        </span>
      </div>

      {/* 右侧:运行状态 + 窗口控制 */}
      <div class="flex items-center gap-[12px]">
        {/* 运行状态指示:有实例运行时绿点 + 计数,否则灰点 + 无运行 */}
        <div class="flex items-center gap-[6px]" data-tauri-drag-region>
          <Show
            when={!instances.loading}
            fallback={<span class="text-[12px] text-dim whitespace-nowrap">载入中…</span>}
          >
            <Show
              when={runningCount() > 0}
              fallback={
                <>
                  <span class="w-[8px] h-[8px] rounded-full shrink-0 bg-n-6" aria-hidden="true" />
                  <span class="text-[12px] text-dim whitespace-nowrap">无实例运行</span>
                </>
              }
            >
              <span
                class="w-[8px] h-[8px] rounded-full shrink-0 bg-a-5 shadow-[0_0_0_3px_color-mix(in_srgb,var(--a-4)_25%,transparent)]"
                aria-hidden="true"
              />
              <span class="text-[12px] text-fg whitespace-nowrap">{runningCount()} 个实例运行中</span>
            </Show>
          </Show>
        </div>

        {/* 窗口控制:no-drag,调 Tauri window API。原生交通灯按钮已提供,这里隐藏自绘控制以免重复。 */}
        <div class="hidden items-center gap-[2px] [-webkit-app-region:no-drag]">
          <button
            class="w-[30px] h-[30px] border-none bg-transparent rounded-ctl text-n-7 cursor-pointer grid place-items-center transition-[background-color,color] duration-[var(--dur)] ease-app motion-reduce:transition-none hover:bg-glass-hover hover:text-fg"
            title="最小化"
            aria-label="最小化"
            onClick={() => windowAction((w) => w.minimize())}
          >
            <MinimizeIcon />
          </button>
          <button
            class="w-[30px] h-[30px] border-none bg-transparent rounded-ctl text-n-7 cursor-pointer grid place-items-center transition-[background-color,color] duration-[var(--dur)] ease-app motion-reduce:transition-none hover:bg-[#e5484d] hover:text-white"
            title="关闭"
            aria-label="关闭"
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
