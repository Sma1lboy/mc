/**
 * 全局键盘快捷键:一处注册 keydown 监听,按平台(mac=⌘ / 其它=Ctrl)分发到 store 动作。
 *
 * 方案:
 *   - ⌘/Ctrl + 1‥9        → 启动第 N 个最近游玩的实例(sortByRecent + playInstance,沿用未装/启动中守卫)
 *   - ⌘/Ctrl + ⇧ + H/L/D/S → 首页 / 库 / 发现 / 设置
 *   - ⌘/Ctrl + ,           → 设置
 *   - ?(⇧+/)无输入聚焦时 → 切换快捷键帮助浮层
 *   - Esc                  → 仅当帮助浮层打开时关闭(否则不吞,交给各页自己的 Esc)
 *
 * 守卫:目标是 INPUT/TEXTAREA/SELECT 或 contentEditable 时一律不触发(放心打字);
 * 不 preventDefault 我们不处理的组合,避免抢浏览器/系统快捷键。
 */
import {
  setCurrentPage,
  instances,
  playInstance,
  shortcutsHelpOpen,
  setShortcutsHelpOpen,
} from "../store";
import { sortByRecent } from "./instances";

/** 平台主修饰键:mac 用 ⌘(metaKey),其它平台用 Ctrl。 */
export function isMac(): boolean {
  if (typeof navigator === "undefined") return false;
  // navigator.platform 已弃用但仍是最稳的同步平台判定;userAgentData 不一定可用。
  return /mac/i.test(navigator.platform) || /mac/i.test(navigator.userAgent);
}

/** 主修饰键是否按下(mac=⌘,其它=Ctrl),且不混入另一侧(避免误触系统组合)。 */
function primaryMod(e: KeyboardEvent): boolean {
  return isMac() ? e.metaKey && !e.ctrlKey : e.ctrlKey && !e.metaKey;
}

/** 焦点是否落在可输入元素上(此时所有快捷键都让位给打字)。 */
function typingInTarget(e: KeyboardEvent): boolean {
  const el = e.target as HTMLElement | null;
  if (!el) return false;
  if (el.isContentEditable) return true;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

function launchRecent(n: number): void {
  const list = sortByRecent(instances() ?? []);
  const target = list[n - 1];
  if (target) void playInstance(target.id); // playInstance 自带未安装 / 启动中守卫
}

function onKeyDown(e: KeyboardEvent): void {
  // Esc:只在帮助浮层打开时拦下关闭;其它情况完全不碰,交给页面级 Esc。
  if (e.key === "Escape") {
    if (shortcutsHelpOpen()) {
      e.preventDefault();
      setShortcutsHelpOpen(false);
    }
    return;
  }

  if (typingInTarget(e)) return;

  // ?(⇧+/):无输入聚焦时切换帮助。不要求主修饰键。
  if (e.key === "?" && !e.metaKey && !e.ctrlKey && !e.altKey) {
    e.preventDefault();
    setShortcutsHelpOpen(!shortcutsHelpOpen());
    return;
  }

  if (!primaryMod(e) || e.altKey) return;

  // ⌘/Ctrl + ,  → 设置(无 Shift)
  if (e.key === "," && !e.shiftKey) {
    e.preventDefault();
    setCurrentPage("settings");
    return;
  }

  // ⌘/Ctrl + 1‥9 → 启动第 N 个最近实例(无 Shift)
  if (!e.shiftKey && /^[1-9]$/.test(e.key)) {
    e.preventDefault();
    launchRecent(Number(e.key));
    return;
  }

  // ⌘/Ctrl + ⇧ + H/L/D/S → 导航(用 e.code 规避 Shift 改写后的字符差异)
  if (e.shiftKey) {
    const nav: Record<string, "home" | "library" | "discover" | "settings"> = {
      KeyH: "home",
      KeyL: "library",
      KeyD: "discover",
      KeyS: "settings",
    };
    const page = nav[e.code];
    if (page) {
      e.preventDefault();
      setCurrentPage(page);
    }
  }
}

/**
 * 注册全局快捷键监听,返回解除函数(在 onCleanup 里调用)。
 * 仅在浏览器环境生效(SSR / 无 window 时空操作)。
 */
export function registerGlobalShortcuts(): () => void {
  if (typeof window === "undefined") return () => {};
  window.addEventListener("keydown", onKeyDown);
  return () => window.removeEventListener("keydown", onKeyDown);
}
