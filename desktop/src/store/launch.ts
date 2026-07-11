import { api, onGameExit, onGameStarted } from "../ipc/api";
import { toast } from "../components/Toast";
import { t } from "../i18n";
import { get, set } from "./state";
import { activeRoot, setCrashReport } from "./app";

// ===== 运行中的游戏(进程生命周期) =====
// 维护一份全局「正在运行的实例 id」集合。组件响应式读取:
//   useAppStore((s) => s.runningIds.has(id))
// 非组件代码用 isRunning(id) 取快照。

/** 某实例当前是否在运行(快照;组件请用 useAppStore 订阅)。 */
export function isRunning(id: string): boolean {
  return get().runningIds.has(id);
}

/** 正在运行的实例 id 集合(快照)。 */
export const runningIds = (): ReadonlySet<string> => get().runningIds;

function markRunning(id: string, running: boolean): void {
  const prev = get().runningIds;
  if (running === prev.has(id)) return; // 无变化,保持引用稳定
  const next = new Set(prev);
  if (running) next.add(id);
  else next.delete(id);
  set({ runningIds: next });
}

/** 某实例是否正在启动(已点 Play 但进程尚未确认起来;快照)。 */
export function isLaunching(id: string): boolean {
  return get().launchingIds.has(id);
}

function markLaunching(id: string, on: boolean): void {
  const prev = get().launchingIds;
  if (on === prev.has(id)) return;
  const next = new Set(prev);
  if (on) next.add(id);
  else next.delete(id);
  set({ launchingIds: next });
}

/**
 * 统一的「启动 / 停止」入口:运行中→停止;否则启动并守卫重复点击。
 * `server` 为可选的一次性进入服务器(`host` 或 `host:port`),仅本次启动生效。
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
  // 领域薄存根(加入但未「开始同步」装核心)不可启动:引导去实例里点开始同步。
  const summary = (get().instances ?? []).find((x) => x.id === id);
  if (summary && !summary.installed) {
    toast({ type: "info", message: t("store.launch.pendingRealm") });
    return;
  }
  markLaunching(id, true);
  try {
    // 用当前选中账号启动。
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
  void api
    .runningInstances()
    .then((ids) => set({ runningIds: new Set(ids) }))
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
      const summary = (get().instances ?? []).find((x) => x.id === e.id);
      const reason =
        e.reason ||
        (e.code != null
          ? t("store.launch.crashedWithCode", { code: e.code })
          : t("store.launch.crashed"));
      setCrashReport({
        id: e.id,
        name: summary?.name ?? e.id,
        mcVersion: summary?.mc_version,
        loader: summary?.loader,
        loaderVersion: summary?.loader_version,
        code: e.code,
        category: e.category,
        reason,
        suggestions: e.suggestions ?? [],
        matched: e.matched,
        logTail: e.log_tail ?? "",
      });
    }
  });
}
