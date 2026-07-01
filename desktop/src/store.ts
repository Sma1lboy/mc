import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { api, onGameExit, onGameStarted } from "./ipc/api";
import { toast } from "./components/Toast";
import { t } from "./i18n";
import type { ProjectKind } from "./ipc/types";
import type { AuthUser, UserBrief, Identity, Notification, InstanceSummary } from "./ipc/bindings";

// 页面标识。home/library/discover/agent/settings + 实例详情。
export type Page = "home" | "library" | "discover" | "agent" | "settings" | "instance";

/**
 * 全局轻量状态:一份 zustand store 持有所有数据字段;读写走本模块导出的
 * getter/setter 函数,与旧的模块级信号写法逐一对齐。
 *
 * 约定(见 MIGRATION.md):
 *   - 组件里要「响应式」读某字段 → 用 hook:`useAppStore((s) => s.currentPage)`。
 *   - 非组件代码(本模块内部、事件回调、工具函数)→ 用 getter:`currentPage()`
 *     (读 `getState()`,不订阅,取当下快照即可)。
 * 两条路读同一份 store,永不分叉。
 */

// ===== 崩溃报告 =====
// 游戏异常退出(非零 / 被信号杀死)时,后端 game://exit 带回诊断结果,这里组装成一份
// 可读的崩溃报告交给全局 <CrashDialog/> 展示。正常退出 / 用户主动停止不触发(report 保持 null)。
export interface CrashReport {
  id: string;
  /** 实例名(从 instances 查得,查不到回落 id)。 */
  name: string;
  mcVersion?: string;
  loader?: string;
  loaderVersion?: string | null;
  /** 退出码(被信号杀死时为 null)。 */
  code: number | null;
  /** 崩溃类别 slug(映射 crash.cat.<slug> 标签);诊断命中才有。 */
  category: string | null;
  /** 人话原因(诊断命中才有)。 */
  reason: string | null;
  /** 可执行建议。 */
  suggestions: string[];
  /** 命中的关键日志行(证据)。 */
  matched: string | null;
  /** 保留的日志尾部。 */
  logTail: string;
}

/** Discover 跳转目标(首页卡片 → Discover 自动打开某项目详情)。 */
export interface DiscoverTarget {
  hit: import("./components/ModpackCard").ModpackHit;
  kind: import("./ipc/types").ProjectKind;
}

/** 检查更新的结果(某实例有几个 mod 可更新 / 整合包是否有新版)。 */
export type InstanceUpdateState = { mods: number; modpack: boolean };

/** store 的全部数据字段(仅数据;setter/action 为本模块导出的函数)。 */
interface AppState {
  currentPage: Page;
  shortcutsHelpOpen: boolean;
  crashReport: CrashReport | null;
  discoverKind: ProjectKind;
  currentRoot: string | null;
  /** 全局实例列表;undefined = 尚未加载(对齐旧 resource 的「未就绪」语义)。 */
  instances: InstanceSummary[] | undefined;
  updatesByInstance: Record<string, InstanceUpdateState>;
  checkingUpdates: boolean;
  currentInstanceId: string | null;
  instanceReturnPage: Page;
  discoverTarget: DiscoverTarget | null;
  veilStrength: number;
  runningIds: ReadonlySet<string>;
  launchingIds: ReadonlySet<string>;
  kobeUser: AuthUser | null;
  socialEnabled: boolean;
  friends: UserBrief[];
  friendRequests: UserBrief[];
  accountIdentities: Identity[];
  notifications: Notification[];
}

/** Discover 顶栏类型标签的顺序。 */
export const DISCOVER_KINDS: ProjectKind[] = ["modpack", "mod", "shader", "resourcepack", "datapack"];

/** 当前选中的游戏根目录持久化键。 */
const ROOT_STORAGE_KEY = "mc-launcher.current-root";
const VEIL_STORAGE_KEY = "mc-launcher.veil-strength";

function readInitialRoot(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(ROOT_STORAGE_KEY);
  } catch {
    return null;
  }
}

function readInitialVeil(): number {
  if (typeof window === "undefined") return 0.72;
  try {
    const n = parseFloat(window.localStorage.getItem(VEIL_STORAGE_KEY) ?? "");
    return Number.isFinite(n) ? Math.min(1, Math.max(0.3, n)) : 0.72;
  } catch {
    return 0.72;
  }
}

/**
 * 应用 store(单一真相)。组件用它做响应式订阅:`useAppStore((s) => s.instances)`。
 * subscribeWithSelector 让本模块能只订阅某个字段(kobeUser 登录/登出副作用)。
 */
export const useAppStore = create<AppState>()(
  subscribeWithSelector((): AppState => ({
    currentPage: "home",
    shortcutsHelpOpen: false,
    crashReport: null,
    discoverKind: "modpack",
    currentRoot: readInitialRoot(),
    instances: undefined,
    updatesByInstance: {},
    checkingUpdates: false,
    currentInstanceId: null,
    instanceReturnPage: "home",
    discoverTarget: null,
    veilStrength: readInitialVeil(),
    runningIds: new Set<string>(),
    launchingIds: new Set<string>(),
    kobeUser: null,
    socialEnabled: true,
    friends: [],
    friendRequests: [],
    accountIdentities: [],
    notifications: [],
  })),
);

// 快照读取的简写(非组件代码用)。
const get = useAppStore.getState;
const set = useAppStore.setState;

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

// ===== kobeMC 账号(我们自己的后端账号,区别于游戏内 MC 账号) =====
export const kobeUser = (): AuthUser | null => get().kobeUser;

/** 是否已登录 kobeMC 账号(快照)。 */
export function isKobeSignedIn(): boolean {
  return get().kobeUser !== null;
}

// ===== 在线状态心跳(presence) =====
/** 当前活动文案:取任一正在运行的实例名;无则 null(空闲)。 */
function currentActivity(): string | null {
  const running = get().runningIds;
  if (running.size === 0) return null;
  const inst = (get().instances ?? []).find((x) => running.has(x.id));
  return inst?.name ?? null;
}

/** 上报一次心跳(仅在已登录时)。 */
export function sendPresenceHeartbeat(): void {
  if (!isKobeSignedIn()) return;
  void api.presenceHeartbeat(currentActivity()).catch(() => {});
}

if (typeof window !== "undefined") {
  setInterval(sendPresenceHeartbeat, 60_000);
}

/** 邮箱 + 密码登录 kobeMC 账号;成功后填充全局会话。 */
export async function kobeLogin(email: string, password: string): Promise<void> {
  const user = await api.kobemcLogin(email, password);
  set({ kobeUser: user });
  sendPresenceHeartbeat();
  toast({ type: "info", message: t("kobe.toast.loggedIn", { name: kobeDisplayName(user) }) });
}

/**
 * 注册新 kobeMC 账号(注册即登录,沿用同一会话 cookie)。
 * 设好友用户名失败(如已被占用)不阻断登录:toast 提示后照常保持登录态。
 */
export async function kobeSignup(email: string, password: string, username: string): Promise<void> {
  const user = await api.kobemcSignup(email, password, username);
  set({ kobeUser: user });
  sendPresenceHeartbeat();
  toast({ type: "info", message: t("kobe.toast.signedUp", { name: kobeDisplayName(user) }) });
  try {
    await api.friendSetUsername(username);
  } catch {
    toast({ type: "error", message: t("kobe.usernameTaken") });
  }
  await refreshKobeUser();
}

/** 退出 kobeMC 账号(清后端会话 + 本地状态)。 */
export async function kobeLogout(): Promise<void> {
  const email = get().kobeUser?.email ?? null;
  try {
    await api.kobemcLogout();
  } finally {
    set({ kobeUser: null });
  }
  // 显式退出后下次启动不再自动登录该账号;凭据仍留在列表里(只关掉它的 auto_login)。
  if (email) {
    try {
      await api.kobeSetAutoLogin(email, false);
    } catch {
      /* 忽略 */
    }
  }
}

/** 账号展示名:优先昵称 → 用户名 → 邮箱 → id 前缀。 */
export function kobeDisplayName(user: AuthUser): string {
  return user.name || user.username || user.email || user.id.slice(0, 8);
}

/** 重新拉取后端会话用户(设用户名/资料变更后刷新 kobeUser)。 */
export async function refreshKobeUser(): Promise<void> {
  try {
    set({ kobeUser: (await api.kobemcSession()) ?? null });
  } catch {
    /* 会话探测失败不影响现有登录态 */
  }
}

// 启动时探一次后端会话:有效则恢复登录态;否则(全新进程)若记住凭据且开了自动登录,静默登录。
if (typeof window !== "undefined") {
  void api
    .kobemcSession()
    .then(async (user) => {
      if (user) {
        set({ kobeUser: user });
        sendPresenceHeartbeat();
        return;
      }
      try {
        const auto = (await api.kobeListCredentials()).find((c) => c.auto_login);
        if (auto?.email && auto.password) {
          await kobeLogin(auto.email, auto.password);
        }
      } catch {
        /* 自动登录尽力而为:失败(密码改了 / 网络)保持登出态,不打扰 */
      }
    })
    .catch(() => {});
}

// ===== 社交 UI 可见性(kobeMC 账号 / 领域 / 好友) =====
export const socialEnabled = (): boolean => get().socialEnabled;

if (typeof window !== "undefined") {
  void api
    .socialEnabled()
    .then((on) => set({ socialEnabled: on }))
    .catch(() => {});
}

/** 设置社交 UI 可见性并持久化(写 social_enabled 显式覆盖)。 */
export async function setSocialEnabled(on: boolean): Promise<void> {
  set({ socialEnabled: on });
  try {
    const s = await api.getSettings();
    await api.setSettings({ ...s, social_enabled: on });
  } catch {
    /* 持久化失败不影响本次会话的显示状态 */
  }
}

// ===== 社交数据缓存(好友 / 好友请求 / 关联身份 / 通知) =====
export const friends = (): UserBrief[] => get().friends;
export const friendRequests = (): UserBrief[] => get().friendRequests;
export const accountIdentities = (): Identity[] => get().accountIdentities;
export const notifications = (): Notification[] => get().notifications;

/** 刷新好友列表(含在线状态/活动);未登录或社交关闭时不动。 */
export async function refreshFriends(): Promise<void> {
  if (!isKobeSignedIn() || !get().socialEnabled) return;
  try {
    set({ friends: await api.friendList() });
  } catch {
    /* 保留旧值 */
  }
}

/** 刷新收到的好友请求;未登录或社交关闭时不动。 */
export async function refreshFriendRequests(): Promise<void> {
  if (!isKobeSignedIn() || !get().socialEnabled) return;
  try {
    set({ friendRequests: await api.friendRequests() });
  } catch {
    /* 保留旧值 */
  }
}

/** 刷新通知中心列表;未登录或社交关闭时不动。容错:失败保留旧值。 */
export async function refreshNotifications(): Promise<void> {
  if (!isKobeSignedIn() || !get().socialEnabled) return;
  try {
    set({ notifications: await api.notifications() });
  } catch {
    /* 保留旧值 */
  }
}

/** 标记所有通知为已读(打开铃铛下拉时调),随后刷新一次清空角标。 */
export async function markNotificationsRead(): Promise<void> {
  try {
    await api.notificationsRead();
  } catch {
    /* 标记失败不阻断:下次刷新仍按服务端状态显示 */
  }
  await refreshNotifications();
}

/** 刷新当前 kobeMC 账号的关联身份;未登录时不动。 */
export async function refreshIdentities(): Promise<void> {
  if (!isKobeSignedIn()) return;
  try {
    set({ accountIdentities: await api.accountIdentities() });
  } catch {
    /* 保留旧值 */
  }
}

// 应用级副作用:实例首拉、社交轮询、登录/登出社交刷新。仅真实 Tauri 环境。
if (typeof window !== "undefined") {
  // 实例列表首拉(切根由 setCurrentRoot 触发重拉)。
  void refreshInstances();

  // 单一连续轮询:仅在已登录 + 社交开启时刷新好友 + 请求 + 通知(30s 新鲜度)。
  setInterval(() => {
    void refreshFriends();
    void refreshFriendRequests();
    void refreshNotifications();
  }, 30_000);

  // kobeUser 变为已登录 → 拉一次初值;登出 → 清空社交信号。
  // subscribeWithSelector:只在 kobeUser 引用变化时触发(等价旧 createEffect)。
  useAppStore.subscribe(
    (s) => s.kobeUser,
    (user) => {
      if (user) {
        void refreshFriends();
        void refreshFriendRequests();
        void refreshNotifications();
        void refreshIdentities();
      } else {
        set({ friends: [], friendRequests: [], notifications: [], accountIdentities: [] });
      }
    },
  );
}
