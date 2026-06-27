import { createSignal, createResource, createRoot, createEffect } from "solid-js";
import { api, onGameExit, onGameStarted } from "./ipc/api";
import { toast } from "./components/Toast";
import { t } from "./i18n";
import type { ProjectKind } from "./ipc/types";
import type { AuthUser, UserBrief, Identity } from "./ipc/bindings";

// 页面标识。home/library/discover/settings + 实例详情。
export type Page = "home" | "library" | "discover" | "settings" | "instance";

/**
 * 全局轻量状态:模块级 createSignal,任何组件 import 即读写,无需 Context。
 */

// 当前页面,默认 home。
export const [currentPage, setCurrentPage] = createSignal<Page>("home");

// 键盘快捷键帮助浮层是否打开(由 `?` 切换,Esc 关闭)。挂在 store 里,
// 让全局 keydown 处理器(util/shortcuts.ts)与 AppShell 里的 ShortcutsHelp 共享同一状态。
export const [shortcutsHelpOpen, setShortcutsHelpOpen] = createSignal<boolean>(false);

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

const [crashReport, setCrashReport] = createSignal<CrashReport | null>(null);
export { crashReport, setCrashReport };

// Discover 内容类型:提到 store,让顶栏 TopBar 的类型标签与 Discover 页共享同一状态
//(标签上提到顶栏后,Discover 下方就纯粹是筛选 + 内容)。默认整合包。
export const [discoverKind, setDiscoverKind] = createSignal<ProjectKind>("modpack");
/** Discover 顶栏类型标签的顺序。 */
export const DISCOVER_KINDS: ProjectKind[] = ["modpack", "mod", "shader", "resourcepack", "datapack"];

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

// ===== 实例列表(全局单一真相)=====
// 整个应用只有这一份实例列表:库页、首页、侧栏 rail、安装目标选择器都读 instances(),
// 任何增 / 删 / 装 / 改之后调用 refreshInstances() 统一刷新,避免「一处删了、另一处还显示」
// 的状态分叉。源随 activeRoot() 变化,切换根目录自动重拉。
// 在 createRoot 里建,使这条 app 级 resource 有稳定 owner(模块级 createResource 否则会
// 报「computation created outside a root」)。owner 不释放 = 与应用同生命周期。
const [instances, { refetch: refreshInstances }] = createRoot(() =>
  createResource(
    () => activeRoot(),
    (root) => api.listInstances(root),
  ),
);
export { instances, refreshInstances };

// ===== 批量更新检查(按需,绝不自动跑)=====
// 「检查更新」按钮一次性问 Modrinth:每个实例有几个 mod 可更新、整合包是否有新版。
// 结果存这里(只含至少有一项更新的实例),库页卡片据此点亮「有更新」角标。
// 网络密集 → 仅用户点按钮时触发,启动时不跑。
export type InstanceUpdateState = { mods: number; modpack: boolean };
const [updatesByInstance, setUpdatesByInstance] = createSignal<Record<string, InstanceUpdateState>>({});
export { updatesByInstance };

// 检查进行中(按钮转圈 / 禁用)。
const [checkingUpdates, setCheckingUpdates] = createSignal(false);
export { checkingUpdates };

/** 某实例是否有可用更新(供卡片读取点亮角标)。 */
export function instanceHasUpdate(id: string): boolean {
  return id in updatesByInstance();
}

/** 当前有更新的实例数量(供库页头部摘要)。 */
export function updatedInstanceCount(): number {
  return Object.keys(updatesByInstance()).length;
}

/**
 * 一次性检查当前根目录下所有实例的更新,填充 updatesByInstance。
 * 按需调用(用户点「检查更新」),不在启动时自动运行。后端内部有界并发推进、
 * 单实例失败被跳过;这里只负责触发 + 落结果 + 维护 busy 态。
 */
export async function checkAllUpdates(): Promise<void> {
  if (checkingUpdates()) return;
  setCheckingUpdates(true);
  try {
    const list = await api.checkAllUpdates(activeRoot());
    const next: Record<string, InstanceUpdateState> = {};
    for (const u of list) {
      next[u.instance_id] = { mods: u.mod_updates, modpack: u.modpack_update };
    }
    setUpdatesByInstance(next);
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
    setCheckingUpdates(false);
  }
}

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
  // 领域薄存根(加入但未「开始同步」装核心)不可启动:引导去实例里点开始同步。
  const summary = (instances() ?? []).find((x) => x.id === id);
  if (summary && !summary.installed) {
    toast({ type: "info", message: t("store.launch.pendingRealm") });
    return;
  }
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
      // 异常退出:弹出崩溃报告弹窗(可读摘要 + 建议 + 日志尾部 + 复制诊断)。
      const summary = (instances() ?? []).find((x) => x.id === e.id);
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
// 与游戏账号(offline / Microsoft / Yggdrasil)正交:这是登录我们 mc-server 后端的账号,
// 解锁临时领域(realms)同步等服务端能力。会话存活在后端 ServerClient 的 cookie jar 里
//(进程内),因此当前仅维持本次 app 运行;重启需重新登录(MVP 限制,后续可持久化 token)。
const [kobeUser, setKobeUser] = createSignal<AuthUser | null>(null);
export { kobeUser };

/** 是否已登录 kobeMC 账号(响应式)。 */
export function isKobeSignedIn(): boolean {
  return kobeUser() !== null;
}

// ===== 在线状态心跳(presence) =====
// 登录 kobeMC 后周期性上报「我在线 + 在玩什么」,好友列表据此显示在线点 + 活动行。
// 活动 = 当前正在运行的某个实例名(运行领域/实例的名字),空闲则为 null。
// 心跳是 best-effort:失败静默(网络抖动/会话过期不应打扰用户)。

/** 当前活动文案:取任一正在运行的实例名;无则 null(空闲)。 */
function currentActivity(): string | null {
  const running = runningIds();
  if (running.size === 0) return null;
  const inst = (instances() ?? []).find((x) => running.has(x.id));
  return inst?.name ?? null;
}

/** 上报一次心跳(仅在已登录时)。 */
function sendPresenceHeartbeat(): void {
  if (!isKobeSignedIn()) return;
  void api.presenceHeartbeat(currentActivity()).catch(() => {});
}

export { sendPresenceHeartbeat };

if (typeof window !== "undefined") {
  setInterval(sendPresenceHeartbeat, 60_000);
}

/** 邮箱 + 密码登录 kobeMC 账号;成功后填充全局会话。 */
export async function kobeLogin(email: string, password: string): Promise<void> {
  const user = await api.kobemcLogin(email, password);
  setKobeUser(user);
  sendPresenceHeartbeat();
  toast({ type: "info", message: t("kobe.toast.loggedIn", { name: kobeDisplayName(user) }) });
}

/**
 * 注册新 kobeMC 账号(注册即登录,沿用同一会话 cookie)。
 * username 同时作为 better-auth 的展示名(name)与好友用户名 —— 单一身份。
 * 设好友用户名失败(如已被占用)不阻断登录:toast 提示后照常保持登录态,
 * 用户可在好友面板的兜底设名处重设(老/登录账号同走那条兜底路径)。
 */
export async function kobeSignup(email: string, password: string, username: string): Promise<void> {
  const user = await api.kobemcSignup(email, password, username);
  setKobeUser(user);
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
  try {
    await api.kobemcLogout();
  } finally {
    setKobeUser(null);
  }
}

/** 账号展示名:优先昵称 → 用户名 → 邮箱 → id 前缀。 */
export function kobeDisplayName(user: AuthUser): string {
  return user.name || user.username || user.email || user.id.slice(0, 8);
}

// 启动时探一次后端会话:若 cookie jar 仍有效(同一进程的热重载)则恢复登录态。
if (typeof window !== "undefined") {
  void api
    .kobemcSession()
    .then((user) => {
      if (user) {
        setKobeUser(user);
        sendPresenceHeartbeat();
      }
    })
    .catch(() => {});
}

// ===== 社交 UI 可见性(kobeMC 账号 / 领域 / 好友) =====
// 默认按部署场景(便携·和实例同级 → 关;桌面独立版 → 开),设置里可手动覆盖。
// 启动时从后端取生效值;关闭后顶栏账号 chip、领域、好友等社交入口全部隐藏。
const [socialEnabled, setSocialEnabledSig] = createSignal<boolean>(true);
export { socialEnabled };

if (typeof window !== "undefined") {
  void api
    .socialEnabled()
    .then(setSocialEnabledSig)
    .catch(() => {});
}

/** 设置社交 UI 可见性并持久化(写 social_enabled 显式覆盖)。 */
export async function setSocialEnabled(on: boolean): Promise<void> {
  setSocialEnabledSig(on);
  try {
    const s = await api.getSettings();
    await api.setSettings({ ...s, social_enabled: on });
  } catch {
    /* 持久化失败不影响本次会话的显示状态 */
  }
}

/** 重新拉取后端会话用户(设用户名/资料变更后刷新 kobeUser)。 */
export async function refreshKobeUser(): Promise<void> {
  try {
    setKobeUser((await api.kobemcSession()) ?? null);
  } catch {
    /* 会话探测失败不影响现有登录态 */
  }
}

// ===== 社交数据缓存(好友 / 好友请求 / 关联身份) =====
// 单一真相:顶栏账号下拉的好友/关联区与领域面板的「邀请好友」共享同一份数据,
// 避免每次打开下拉都重新发起 kobe-server 往返、空白闪烁、并重复 friendList 拉取。
// 由 store 持有一条连续 30s 轮询(只在已登录 + 社交开启时刷新好友/请求的在线状态/活动),
// 不随组件挂载重启。登录变化时拉一次初值,登出清空。各刷新器容错:失败保留旧值。
const [friends, setFriends] = createSignal<UserBrief[]>([]);
export { friends };
const [friendRequests, setFriendRequests] = createSignal<UserBrief[]>([]);
export { friendRequests };
const [accountIdentities, setAccountIdentities] = createSignal<Identity[]>([]);
export { accountIdentities };

/** 刷新好友列表(含在线状态/活动);未登录或社交关闭时不动。 */
export async function refreshFriends(): Promise<void> {
  if (!isKobeSignedIn() || !socialEnabled()) return;
  try {
    setFriends(await api.friendList());
  } catch {
    /* 保留旧值 */
  }
}

/** 刷新收到的好友请求;未登录或社交关闭时不动。 */
export async function refreshFriendRequests(): Promise<void> {
  if (!isKobeSignedIn() || !socialEnabled()) return;
  try {
    setFriendRequests(await api.friendRequests());
  } catch {
    /* 保留旧值 */
  }
}

/** 刷新当前 kobeMC 账号的关联身份;未登录时不动。 */
export async function refreshIdentities(): Promise<void> {
  if (!isKobeSignedIn()) return;
  try {
    setAccountIdentities(await api.accountIdentities());
  } catch {
    /* 保留旧值 */
  }
}

if (typeof window !== "undefined") {
  // 单一连续轮询:仅在已登录 + 社交开启时刷新好友 + 请求(30s 新鲜度)。
  // store 持有,绝不随组件挂载/卸载重启,避免多处各自起轮询。
  setInterval(() => {
    void refreshFriends();
    void refreshFriendRequests();
  }, 30_000);

  // kobeUser 变为已登录 → 拉一次初值;登出 → 清空社交信号。
  createRoot(() => {
    createEffect(() => {
      if (kobeUser()) {
        void refreshFriends();
        void refreshFriendRequests();
        void refreshIdentities();
      } else {
        setFriends([]);
        setFriendRequests([]);
        setAccountIdentities([]);
      }
    });
  });
}
