import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import type { ProjectKind } from "../ipc/types";
import type { AuthUser, UserBrief, Identity, Notification, InstanceSummary } from "../ipc/bindings";

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
  hit: import("../components/ModpackCard").ModpackHit;
  kind: import("../ipc/types").ProjectKind;
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
export const ROOT_STORAGE_KEY = "mc-launcher.current-root";
export const VEIL_STORAGE_KEY = "mc-launcher.veil-strength";

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

// 快照读取的简写(非组件代码 / store 内部模块用)。
export const get = useAppStore.getState;
export const set = useAppStore.setState;
