import { Component, Show, createResource, createMemo, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { currentRoot } from "../store";
import type { InstanceSummary } from "../ipc/types";

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

  return (
    // data-tauri-drag-region:让顶栏空白处可拖动窗口
    <header
      class="[grid-area:topbar] h-[48px] flex items-center justify-end glass-panel border-b border-glass-divider pl-[14px] pr-[8px] box-border select-none"
      data-tauri-drag-region
    >

      {/* 右侧:运行状态(玻璃药丸,看起来是个组件而非漂浮文字)+ 品牌名 + 窗口控制 */}
      <div class="flex items-center gap-[10px]">
        <div
          class="inline-flex items-center gap-[6px] h-[26px] pl-[9px] pr-[11px] rounded-full bg-glass-card border border-glass-border"
          data-tauri-drag-region
        >
          <Show
            when={!instances.loading}
            fallback={<span class="text-[12px] text-dim whitespace-nowrap">载入中…</span>}
          >
            <Show
              when={runningCount() > 0}
              fallback={
                <>
                  <span class="w-[7px] h-[7px] rounded-full shrink-0 bg-n-6" aria-hidden="true" />
                  <span class="text-[12px] text-dim whitespace-nowrap">无实例运行</span>
                </>
              }
            >
              <span class="w-[7px] h-[7px] rounded-full shrink-0 bg-a-5" aria-hidden="true" />
              <span class="text-[12px] text-fg whitespace-nowrap">{runningCount()} 个运行中</span>
            </Show>
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
            title="最小化"
            aria-label="最小化"
            onClick={() => windowAction((w) => w.minimize())}
          >
            <MinimizeIcon />
          </button>
          <button
            class="w-[30px] h-[30px] border-none bg-transparent rounded-ctl text-n-7 cursor-pointer grid place-items-center transition-[background-color,color] duration-[var(--dur)] ease-app motion-reduce:transition-none hover:bg-danger hover:text-white"
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
