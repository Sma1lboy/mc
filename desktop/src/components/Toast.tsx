import { create } from "zustand";
import { Presence, useEntrance } from "../motion/react";
import { t, useLang } from "../i18n";

// Toast 类型与配色:
//   info    —— 中性提示 (熔岩橙 accent)
//   success —— 成功 (熔岩橙 accent)
//   warn    —— 警告 (沙金 tag)
//   error   —— 错误 (danger)
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
  /** 是否仍在场;置 false 触发 <Presence> 退场动画,播完才从 store 删除。 */
  open: boolean;
}

// ---- 全局单例 store ----
// 用 module 级 zustand store 实现"任意位置 import { toast } 即可弹"的全局通道,
// 无需 Context Provider。<ToastContainer/> 用 useToastStore 订阅同一份 store。
const useToastStore = create<{ items: ToastItem[] }>(() => ({ items: [] }));
let nextId = 1;

// 记录每个 toast 的定时器, 卸载/手动关闭时清理, 避免泄漏。
const timers = new Map<number, ReturnType<typeof setTimeout>>();

function clearTimer(id: number) {
  const timer = timers.get(id);
  if (timer !== undefined) {
    clearTimeout(timer);
    timers.delete(id);
  }
}

// 真正从 store 移除 (<Presence> 退场动画播完后由 onExited 调用)。
function removeToast(id: number) {
  clearTimer(id);
  useToastStore.setState((s) => ({ items: s.items.filter((it) => it.id !== id) }));
}

// 触发退场:置 open=false,由每个 toast 外层的 <Presence> 播放退场动画,
// 播完后经 onExited→removeToast 从 store 删除。不再手写 setTimeout 与 CSS 时长对齐。
function dismissToast(id: number) {
  clearTimer(id);
  useToastStore.setState((s) => ({
    items: s.items.map((it) => (it.id === id ? { ...it, open: false } : it)),
  }));
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
    open: true,
  };
  useToastStore.setState((s) => ({ items: [...s.items, item] }));

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
toast.error = (message: string, duration?: number) => toast({ type: "error", message, duration });

// 类型 → 左色条颜色 (border-left-color) 的工具类。info/success 用熔岩橙 accent。
function toastBorderClass(type: ToastType): string {
  switch (type) {
    case "warn":
      return "border-l-tag";
    case "error":
      return "border-l-danger";
    default: // info / success
      return "border-l-accent";
  }
}

// 类型 → 图标颜色 (currentColor)。与左色条一致。
function toastIconColorClass(type: ToastType): string {
  switch (type) {
    case "warn":
      return "text-tag";
    case "error":
      return "text-danger-text";
    default: // info / success
      return "text-accent";
  }
}

// 每种类型的图标 (内联 SVG, currentColor 跟随类型色)。
function ToastIcon({ type }: { type: ToastType }) {
  return (
    <span className={`shrink-0 flex ${toastIconColorClass(type)}`}>
      {type === "success" ? (
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <circle cx="8" cy="8" r="7" stroke="currentColor" strokeWidth="1.5" />
          <path
            d="m5 8 2 2 4-4"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      ) : type === "error" ? (
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <circle cx="8" cy="8" r="7" stroke="currentColor" strokeWidth="1.5" />
          <path d="M8 4.5v4M8 11h.01" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      ) : type === "warn" ? (
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <path
            d="M7.13 2.5a1 1 0 0 1 1.74 0l5.2 9.2A1 1 0 0 1 13.2 13.2H2.8a1 1 0 0 1-.87-1.5l5.2-9.2Z"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinejoin="round"
          />
          <path d="M8 6v2.5M8 10.6h.01" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      ) : (
        // info 默认
        <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <circle cx="8" cy="8" r="7" stroke="currentColor" strokeWidth="1.5" />
          <path d="M8 7v4M8 5h.01" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
        </svg>
      )}
    </span>
  );
}

// 单个 toast 卡片:用 useEntrance("toast") 挂载即播入场(替换 use:motion 指令)。
function ToastCard({ item }: { item: ToastItem }) {
  const ref = useEntrance("toast");
  return (
    <div
      ref={ref}
      className={
        "pointer-events-auto flex items-center gap-[10px] min-w-[240px] max-w-[380px] " +
        "px-[14px] py-[11px] rounded-none bg-panel text-fg border border-titlebar shadow-raised " +
        "border-l-4 text-[13px] leading-[1.4] origin-center " +
        toastBorderClass(item.type)
      }
      role="status"
    >
      <ToastIcon type={item.type} />
      <span className="flex-1 min-w-0 break-words">{item.message}</span>
      <button
        type="button"
        className={
          "shrink-0 inline-flex items-center justify-center w-[18px] h-[18px] " +
          "border-none bg-transparent text-muted cursor-pointer rounded-none " +
          "transition-[color,background-color] duration-[var(--dur)] ease-app " +
          "hover:text-fg hover:bg-panel-3"
        }
        aria-label={t("components.toast.close")}
        onClick={() => dismissToast(item.id)}
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
          <path d="M3 3l6 6M9 3l-6 6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      </button>
    </div>
  );
}

// ToastContainer —— 挂一次在 AppShell 根部。渲染左下角 toast 堆叠。
export function ToastContainer() {
  useLang();
  const items = useToastStore((s) => s.items);
  return (
    <div
      className="fixed right-[16px] bottom-[16px] z-[9999] flex flex-col-reverse items-end gap-[10px] pointer-events-none"
      aria-live="polite"
    >
      {items.map((item) => (
        // 每个 toast 自带 <Presence>:open=false 时播 toast 退场预设(微缩+淡出),
        // 播完 onExited 才把条目从 store 删除——取代旧的 setTimeout(220) 对齐 hack。
        <Presence
          key={item.id}
          show={item.open}
          exitPreset="toast"
          onExited={() => removeToast(item.id)}
        >
          <ToastCard item={item} />
        </Presence>
      ))}
    </div>
  );
}

// 不对外暴露内部 store, 页面只通过 toast() 与 <ToastContainer/> 交互。
export default ToastContainer;
