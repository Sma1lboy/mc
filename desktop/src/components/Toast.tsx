import { JSX, For, Switch, Match } from "solid-js";
import { createStore, produce } from "solid-js/store";
import "./Toast.css";

// Toast 类型与配色:
//   info    —— 中性提示 (accent 蓝/绿)
//   success —— 成功 (accent)
//   warn    —— 警告 (黄)
//   error   —— 错误 (#ff5c5c)
export type ToastType = "info" | "success" | "warn" | "error";

export interface ToastOptions {
  type?: ToastType;
  message: string;
  /** 自定义持续毫秒, 默认 3000。传 0 / Infinity 表示不自动消失。 */
  duration?: number;
}

interface ToastItem {
  id: number;
  type: ToastType;
  message: string;
  duration: number;
  leaving: boolean; // 退出动画中
}

// ---- 全局单例 store ----
// 用 module 级 store 实现"任意位置 import { toast } 即可弹"的全局通道,
// 无需 Context Provider。<ToastContainer/> 渲染同一份 store。
const [items, setItems] = createStore<ToastItem[]>([]);
let nextId = 1;

// 记录每个 toast 的定时器, 卸载/手动关闭时清理, 避免泄漏。
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function clearTimer(id: number) {
  const t = timers.get(id);
  if (t !== undefined) {
    clearTimeout(t);
    timers.delete(id);
  }
}

// 真正从 store 移除 (退出动画播完后调用)。
function removeToast(id: number) {
  clearTimer(id);
  setItems((arr) => arr.filter((it) => it.id !== id));
}

// 触发退出动画, 动画结束再移除。
function dismissToast(id: number) {
  clearTimer(id);
  // 标记 leaving → CSS 播放退出动画。
  setItems(
    produce((arr) => {
      const it = arr.find((x) => x.id === id);
      if (it) it.leaving = true;
    })
  );
  // 与 ui-toast-out 动画时长 (200ms) 对齐。
  setTimeout(() => removeToast(id), 220);
}

/**
 * 全局 toast 函数。任意模块 `import { toast } from "@/components"` 后调用。
 * @example toast({ type: "error", message: "启动失败" })
 */
export function toast(opts: ToastOptions): number {
  const id = nextId++;
  const duration = opts.duration ?? 3000;
  const item: ToastItem = {
    id,
    type: opts.type ?? "info",
    message: opts.message,
    duration,
    leaving: false,
  };
  setItems((arr) => [...arr, item]);

  // 自动消失 (duration 有限且 > 0 时)。
  if (Number.isFinite(duration) && duration > 0) {
    const timer = setTimeout(() => dismissToast(id), duration);
    timers.set(id, timer);
  }
  return id;
}

// 便捷方法 (可选语法糖)。
toast.info = (message: string, duration?: number) => toast({ type: "info", message, duration });
toast.success = (message: string, duration?: number) =>
  toast({ type: "success", message, duration });
toast.warn = (message: string, duration?: number) => toast({ type: "warn", message, duration });
toast.error = (message: string, duration?: number) =>
  toast({ type: "error", message, duration });

// 每种类型的图标 (内联 SVG, currentColor 跟随类型色)。
function ToastIcon(props: { type: ToastType }): JSX.Element {
  return (
    <span class="ui-toast__icon">
      <Switch>
        <Match when={props.type === "success"}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <circle cx="8" cy="8" r="7" stroke="currentColor" stroke-width="1.5" />
            <path
              d="m5 8 2 2 4-4"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </Match>
        <Match when={props.type === "error"}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <circle cx="8" cy="8" r="7" stroke="currentColor" stroke-width="1.5" />
            <path
              d="M8 4.5v4M8 11h.01"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
            />
          </svg>
        </Match>
        <Match when={props.type === "warn"}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path
              d="M7.13 2.5a1 1 0 0 1 1.74 0l5.2 9.2A1 1 0 0 1 13.2 13.2H2.8a1 1 0 0 1-.87-1.5l5.2-9.2Z"
              stroke="currentColor"
              stroke-width="1.4"
              stroke-linejoin="round"
            />
            <path
              d="M8 6v2.5M8 10.6h.01"
              stroke="currentColor"
              stroke-width="1.5"
              stroke-linecap="round"
            />
          </svg>
        </Match>
        {/* info 默认 */}
        <Match when={props.type === "info"}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <circle cx="8" cy="8" r="7" stroke="currentColor" stroke-width="1.5" />
            <path
              d="M8 7v4M8 5h.01"
              stroke="currentColor"
              stroke-width="1.6"
              stroke-linecap="round"
            />
          </svg>
        </Match>
      </Switch>
    </span>
  );
}

// ToastContainer —— 挂一次在 AppShell 根部。渲染左下角 toast 堆叠。
export function ToastContainer(): JSX.Element {
  return (
    <div class="ui-toast-container" aria-live="polite">
      <For each={items}>
        {(item) => (
          <div
            class={`ui-toast ui-toast--${item.type}${
              item.leaving ? " ui-toast--leaving" : ""
            }`}
            role="status"
          >
            <ToastIcon type={item.type} />
            <span class="ui-toast__msg">{item.message}</span>
            <button
              type="button"
              class="ui-toast__close"
              aria-label="Dismiss"
              onClick={() => dismissToast(item.id)}
            >
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                <path
                  d="M3 3l6 6M9 3l-6 6"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                />
              </svg>
            </button>
          </div>
        )}
      </For>
    </div>
  );
}

// 不对外暴露内部 store, 页面只通过 toast() 与 <ToastContainer/> 交互。
export default ToastContainer;
