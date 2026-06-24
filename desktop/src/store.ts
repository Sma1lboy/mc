import { createSignal } from "solid-js";
import { api, onGameExit, onGameStarted } from "./ipc/api";
import { toast } from "./components/Toast";
import { t } from "./i18n";

// 页面标识。home/library/discover/settings + 实例详情。
export type Page = "home" | "library" | "discover" | "settings" | "instance";

/**
 * 全局轻量状态:模块级 createSignal,任何组件 import 即读写,无需 Context。
 */

// 当前页面,默认 home。
export const [currentPage, setCurrentPage] = createSignal<Page>("home");

/** 当前选中的游戏根目录(GameRoot.path);null = 未选/未加载。 */
const ROOT_STORAGE_KEY = "mc-launcher.current-root";

function readInitialRoot(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(ROOT_STORAGE_KEY);
  } catch {
    return null;
  }
}

const [currentRoot, setCurrentRootSig] = createSignal<string | null>(readInitialRoot());
export { currentRoot };

/** 设置当前根并持久化(多根/自定义根场景下让选择跨重启保留)。 */
export function setCurrentRoot(path: string | null): void {
  setCurrentRootSig(path);
  if (typeof window === "undefined") return;
  try {
    if (path) window.localStorage.setItem(ROOT_STORAGE_KEY, path);
    else window.localStorage.removeItem(ROOT_STORAGE_KEY);
  } catch {
    /* localStorage 在加固的 WebView 里可能不可用 */
  }
}

/**
 * 传给后端的「当前根」:未选时落到 ""(后端据此用默认根)。所有 IPC 调用与
 * createResource 的 root 源都经此取根,把这个「空根 → 默认根」的约定收敛到一处。
 */
export const activeRoot = (): string => currentRoot() ?? "";

// ===== 实例详情页 =====
// 点击实例进入详情页(currentPage="instance"),记住来源页用于返回。
export const [currentInstanceId, setCurrentInstanceId] = createSignal<string | null>(null);
const [instanceReturnPage, setInstanceReturnPage] = createSignal<Page>("home");

/** 进入某实例的详情页。 */
export function openInstance(id: string): void {
  if (currentPage() !== "instance") setInstanceReturnPage(currentPage());
  setCurrentInstanceId(id);
  setCurrentPage("instance");
}

/** 从详情页返回来源页。 */
export function closeInstance(): void {
  setCurrentPage(instanceReturnPage());
}

// ===== 跳转到「发现」并(可选)自动打开某项目详情 =====
// 首页「发现」卡片点击 → 存目标 + 切到 discover;Discover 挂载后读取并打开,然后清空。
export const [discoverTarget, setDiscoverTarget] = createSignal<{
  hit: import("./components/ModpackCard").ModpackHit;
  kind: import("./ipc/types").ProjectKind;
} | null>(null);

/** 跳到「发现」页;传 target 则自动打开该项目详情。 */
export function openDiscover(target?: {
  hit: import("./components/ModpackCard").ModpackHit;
  kind: import("./ipc/types").ProjectKind;
}): void {
  setDiscoverTarget(target ?? null);
  setCurrentPage("discover");
}

// ===== 界面透明度(窗口面纱)=====
// 0.3(很透)~ 1(实色)。设置页滑块调节,写 CSS 变量 --veil-strength 即时生效,并存 localStorage。
const VEIL_STORAGE_KEY = "mc-launcher.veil-strength";

function readInitialVeil(): number {
  if (typeof window === "undefined") return 0.72;
  try {
    const n = parseFloat(window.localStorage.getItem(VEIL_STORAGE_KEY) ?? "");
    return Number.isFinite(n) ? Math.min(1, Math.max(0.3, n)) : 0.72;
  } catch {
    return 0.72;
  }
}

const [veilStrength, setVeilStrengthSig] = createSignal<number>(readInitialVeil());
export { veilStrength };

/** 设置窗口面纱不透明度(0.3~1),即时写入 CSS 变量并持久化。 */
export function setVeilStrength(v: number): void {
  const clamped = Math.min(1, Math.max(0.3, v));
  setVeilStrengthSig(clamped);
  if (typeof window === "undefined") return;
  document.documentElement.style.setProperty("--veil-strength", String(clamped));
  try {
    window.localStorage.setItem(VEIL_STORAGE_KEY, String(clamped));
  } catch {
    /* localStorage 不可用时忽略 */
  }
}

// 启动即把持久化的透明度写进 CSS 变量(独立于主题注入)。
if (typeof window !== "undefined") {
  document.documentElement.style.setProperty("--veil-strength", String(veilStrength()));
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

// 「正在启动」集合:点 Play 到 game://started 之间的中间态,用于禁用按钮防重复启动。
const [launchingIds, setLaunchingIds] = createSignal<ReadonlySet<string>>(new Set());

/** 某实例是否正在启动(已点 Play 但进程尚未确认起来)。 */
export function isLaunching(id: string): boolean {
  return launchingIds().has(id);
}

function markLaunching(id: string, on: boolean) {
  setLaunchingIds((prev) => {
    if (on === prev.has(id)) return prev;
    const next = new Set(prev);
    if (on) next.add(id);
    else next.delete(id);
    return next;
  });
}

/**
 * 统一的「启动 / 停止」入口:运行中→停止;否则启动并守卫重复点击。
 * Home / Library / 实例详情共用,避免各页各写一份(且各自缺少防抖)。
 * 成功 toast 用「正在启动…」(launchInstance 返回 ≠ 游戏就绪);就绪/退出由事件维护。
 * `server` 为可选的一次性进入服务器(`host` 或 `host:port`),仅本次启动生效,不改实例配置。
 */
export async function playInstance(id: string, server?: string): Promise<void> {
  if (isRunning(id)) {
    try {
      await api.stopInstance(id);
    } catch (e) {
      toast({ type: "error", message: t("store.launch.stopFailed", { error: String(e) }) });
    }
    return;
  }
  if (isLaunching(id)) return; // 防重复启动
  markLaunching(id, true);
  try {
    // 用当前选中账号启动(此前硬编码 "Player"/offline,会无视已登录账号)。
    const accounts = await api.listAccounts().catch(() => []);
    const acc = accounts.find((a) => a.selected) ?? accounts[0];
    const name = acc?.username ?? "Player";
    const online = !!acc && acc.kind !== "offline";
    await api.launchInstance(activeRoot(), id, name, online, server ?? null);
    toast({ type: "info", message: t("store.launch.starting") });
  } catch (e) {
    markLaunching(id, false);
    toast({ type: "error", message: t("store.launch.launchFailed", { error: String(e) }) });
  }
  // 成功时保持 launching=true,直到 game://started(转 running)或 game://exit 清除。
}

// 仅在真实 Tauri 环境(有 window)下挂监听并同步初始运行态。
if (typeof window !== "undefined") {
  // 挂载时同步一次已在运行的实例(热重载 / 页面重建后不丢运行态)。
  void api
    .runningInstances()
    .then((ids) => setRunningIds(new Set(ids)))
    .catch(() => {});

  onGameStarted((e) => {
    markLaunching(e.id, false);
    markRunning(e.id, true);
  });

  onGameExit((e) => {
    markLaunching(e.id, false);
    markRunning(e.id, false);
    if (e.success) {
      toast({ type: "info", message: t("store.launch.exited") });
    } else {
      const reason =
        e.reason ||
        (e.code != null
          ? t("store.launch.crashedWithCode", { code: e.code })
          : t("store.launch.crashed"));
      const hint = e.suggestions && e.suggestions.length > 0 ? ` —— ${e.suggestions[0]}` : "";
      toast({ type: "error", message: `${reason}${hint}` });
    }
  });
}
