import { api } from "../ipc/api";
import { toast } from "../components/Toast";
import { t } from "../i18n";
import type { ProjectKind } from "../ipc/types";
import type { InstanceSummary } from "../ipc/bindings";
import { get, set, ROOT_STORAGE_KEY, VEIL_STORAGE_KEY, type Page, type DiscoverTarget, type InstanceUpdateState, type CrashReport } from "./state";

// ===== 当前页面 =====
export const currentPage = (): Page => get().currentPage;
export function setCurrentPage(page: Page): void {
  set({ currentPage: page });
}

// 键盘快捷键帮助浮层是否打开(由 `?` 切换,Esc 关闭)。
export const shortcutsHelpOpen = (): boolean => get().shortcutsHelpOpen;
export function setShortcutsHelpOpen(open: boolean): void {
  set({ shortcutsHelpOpen: open });
}

// ===== 崩溃报告 =====
export const crashReport = (): CrashReport | null => get().crashReport;
export function setCrashReport(report: CrashReport | null): void {
  set({ crashReport: report });
}

// ===== Discover 内容类型 =====
export const discoverKind = (): ProjectKind => get().discoverKind;
export function setDiscoverKind(kind: ProjectKind): void {
  set({ discoverKind: kind });
}

// ===== 当前游戏根目录 =====
export const currentRoot = (): string | null => get().currentRoot;

/** 设置当前根并持久化;根变化后重拉实例列表(切根自动重拉)。 */
export function setCurrentRoot(path: string | null): void {
  set({ currentRoot: path });
  if (typeof window !== "undefined") {
    try {
      if (path) window.localStorage.setItem(ROOT_STORAGE_KEY, path);
      else window.localStorage.removeItem(ROOT_STORAGE_KEY);
    } catch {
      /* localStorage 在加固的 WebView 里可能不可用 */
    }
  }
  void refreshInstances();
}

/**
 * 传给后端的「当前根」:未选时落到 ""(后端据此用默认根)。所有 IPC 调用都经此取根,
 * 把「空根 → 默认根」的约定收敛到一处。
 */
export const activeRoot = (): string => get().currentRoot ?? "";

// ===== 实例列表(全局单一真相)=====
// 整个应用只有这一份实例列表:库页、首页、侧栏 rail、安装目标选择器都读 instances(),
// 任何增 / 删 / 装 / 改之后调用 refreshInstances() 统一刷新。切根(setCurrentRoot)也会重拉。
export const instances = (): InstanceSummary[] | undefined => get().instances;

/** 用当前根重拉实例列表。失败保留旧值(不清空,避免闪空)。 */
export async function refreshInstances(): Promise<void> {
  try {
    set({ instances: await api.listInstances(activeRoot()) });
  } catch {
    /* 保留旧值 */
  }
}

// ===== 批量更新检查(按需,绝不自动跑)=====
export const updatesByInstance = (): Record<string, InstanceUpdateState> => get().updatesByInstance;
export const checkingUpdates = (): boolean => get().checkingUpdates;

/** 某实例是否有可用更新(供卡片读取点亮角标)。 */
export function instanceHasUpdate(id: string): boolean {
  return id in get().updatesByInstance;
}

/** 当前有更新的实例数量(供库页头部摘要)。 */
export function updatedInstanceCount(): number {
  return Object.keys(get().updatesByInstance).length;
}

/**
 * 一次性检查当前根目录下所有实例的更新,填充 updatesByInstance。
 * 按需调用(用户点「检查更新」),不在启动时自动运行。
 */
export async function checkAllUpdates(): Promise<void> {
  if (get().checkingUpdates) return;
  set({ checkingUpdates: true });
  try {
    const list = await api.checkAllUpdates(activeRoot());
    const next: Record<string, InstanceUpdateState> = {};
    for (const u of list) {
      next[u.instance_id] = { mods: u.mod_updates, modpack: u.modpack_update };
    }
    set({ updatesByInstance: next });
    toast({
      type: list.length > 0 ? "info" : "success",
      message:
        list.length > 0
          ? t("library.updatesFound", { n: list.length })
          : t("library.updatesNone"),
    });
  } catch (e) {
    toast({ type: "error", message: t("library.updatesCheckFailed", { err: String(e) }) });
  } finally {
    set({ checkingUpdates: false });
  }
}

// ===== 实例详情页 =====
export const currentInstanceId = (): string | null => get().currentInstanceId;
export function setCurrentInstanceId(id: string | null): void {
  set({ currentInstanceId: id });
}

/** 进入某实例的详情页(记住来源页用于返回)。 */
export function openInstance(id: string): void {
  const page = get().currentPage;
  set({
    currentInstanceId: id,
    currentPage: "instance",
    instanceReturnPage: page !== "instance" ? page : get().instanceReturnPage,
  });
}

/** 从详情页返回来源页。 */
export function closeInstance(): void {
  set({ currentPage: get().instanceReturnPage });
}

// ===== 跳转到「发现」并(可选)自动打开某项目详情 =====
export const discoverTarget = (): DiscoverTarget | null => get().discoverTarget;
export function setDiscoverTarget(target: DiscoverTarget | null): void {
  set({ discoverTarget: target });
}

/** 跳到「发现」页;传 target 则自动打开该项目详情。 */
export function openDiscover(target?: DiscoverTarget): void {
  set({ discoverTarget: target ?? null, currentPage: "discover" });
}

// ===== 界面透明度(窗口面纱)=====
export const veilStrength = (): number => get().veilStrength;

/** 设置窗口面纱不透明度(0.3~1),即时写入 CSS 变量并持久化。 */
export function setVeilStrength(v: number): void {
  const clamped = Math.min(1, Math.max(0.3, v));
  set({ veilStrength: clamped });
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
  document.documentElement.style.setProperty("--veil-strength", String(get().veilStrength));
}
