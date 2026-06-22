import { createSignal } from "solid-js";
import { saveTheme, themeForLayout } from "./theme/theme";
import { api, onGameExit, onGameStarted } from "./ipc/api";
import { toast } from "./components/Toast";

// 页面标识。工作台布局用 home/library/discover/settings;
// 经典布局用 launch/discover/settings/more(顶部 Tab,带图标)。
export type Page = "home" | "library" | "discover" | "settings" | "launch" | "more";

/** 界面布局风格:工作台视图或经典视图。 */
export type LayoutMode = "workspace" | "classic";

const LAYOUT_MODE_STORAGE_KEY = "mc-launcher.layout-mode";

/**
 * 全局轻量状态:模块级 createSignal,任何组件 import 即读写,无需 Context。
 */

function normalizeLayoutMode(value: unknown): LayoutMode {
  if (value === "workspace" || value === "modrinth") return "workspace";
  if (value === "classic") return "classic";
  return "classic";
}

function readInitialLayoutMode(): LayoutMode {
  if (typeof window === "undefined") return "classic";
  try {
    const raw = window.localStorage.getItem(LAYOUT_MODE_STORAGE_KEY);
    const mode = normalizeLayoutMode(raw);
    if (raw && raw !== mode) {
      window.localStorage.setItem(LAYOUT_MODE_STORAGE_KEY, mode);
    }
    return mode;
  } catch {
    return "classic";
  }
}

function persistLayoutMode(mode: LayoutMode): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LAYOUT_MODE_STORAGE_KEY, mode);
  } catch {
    /* localStorage can be unavailable in hardened WebView contexts. */
  }
}

const initialLayoutMode = readInitialLayoutMode();

// 当前页面(工作台默认 home;切到经典视图时改成 launch)。
export const [currentPage, setCurrentPage] = createSignal<Page>(
  initialLayoutMode === "classic" ? "launch" : "home",
);

/** 当前选中的游戏根目录(GameRoot.path);null = 未选/未加载。 */
export const [currentRoot, setCurrentRoot] = createSignal<string | null>(null);

/**
 * 传给后端的「当前根」:未选时落到 ""(后端据此用默认根)。所有 IPC 调用与
 * createResource 的 root 源都经此取根,把这个「空根 → 默认根」的约定收敛到一处。
 */
export const activeRoot = (): string => currentRoot() ?? "";

/** 界面布局,默认经典视图。 */
export const [layoutMode, setLayoutMode] = createSignal<LayoutMode>(initialLayoutMode);

/**
 * 切换布局,并套用与该布局相称的主题预设,让两种风格各自"对味":
 *   - classic:浅色 + 蓝
 *   - workspace:深色 + 绿
 * 用户之后仍可在设置里单独微调主题色。
 */
export function switchLayout(mode: LayoutMode) {
  setLayoutMode(mode);
  persistLayoutMode(mode);
  void saveTheme(themeForLayout(mode)).catch(() => {});
  setCurrentPage(mode === "classic" ? "launch" : "home");
}

// ===== 运行中的游戏(进程生命周期) =====
// 后端把进程登记进 RunningGames,并通过 game://started / game://exit 广播状态。
// 这里维护一份全局「正在运行的实例 id」集合,任何组件 import isRunning(id) 即可响应式读取
// 运行态(运行点、Play↔Stop 切换)。崩溃/退出的 toast 也在这里统一发,避免各页重复。

const [runningIds, setRunningIds] = createSignal<ReadonlySet<string>>(new Set());

/** 某实例当前是否在运行(响应式)。 */
export function isRunning(id: string): boolean {
  return runningIds().has(id);
}

/** 正在运行的实例 id 集合(响应式)。 */
export { runningIds };

function markRunning(id: string, running: boolean) {
  setRunningIds((prev) => {
    if (running === prev.has(id)) return prev; // 无变化,保持引用稳定
    const next = new Set(prev);
    if (running) next.add(id);
    else next.delete(id);
    return next;
  });
}

// 仅在真实 Tauri 环境(有 window)下挂监听并同步初始运行态。
if (typeof window !== "undefined") {
  // 挂载时同步一次已在运行的实例(热重载 / 页面重建后不丢运行态)。
  void api
    .runningInstances()
    .then((ids) => setRunningIds(new Set(ids)))
    .catch(() => {});

  onGameStarted((e) => markRunning(e.id, true));

  onGameExit((e) => {
    markRunning(e.id, false);
    if (e.success) {
      toast({ type: "info", message: "游戏已退出" });
    } else {
      const reason = e.reason || `游戏异常退出${e.code != null ? `(代码 ${e.code})` : ""}`;
      const hint = e.suggestions && e.suggestions.length > 0 ? ` —— ${e.suggestions[0]}` : "";
      toast({ type: "error", message: `${reason}${hint}` });
    }
  });
}
