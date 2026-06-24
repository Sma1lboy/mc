// IPC 层
// ------------------------------------------------------------------
// 命令直接复用 Rust 经 tauri-specta 生成的 `commands`(./bindings.ts):函数名、参数、
// 返回类型全部由后端单一真相生成,改 Rust 即编译期暴露漂移(杜绝手写 invoke 字符串/类型
// 与后端不一致)。这里只做一件事:把 specta 的「Result 包装」解包回项目既有的
// 「成功 resolve / 失败 throw」调用约定,使页面调用方式(api.xxx(...) + try/catch)不变。
//
// 事件订阅(onXxx)仍手写:事件 payload 不在命令签名里,不由 tauri-specta 收集。
// ------------------------------------------------------------------

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke as rawInvoke } from "@tauri-apps/api/core";
import { commands } from "./bindings";
import type {
  InstallProgress,
  LaunchProgress,
  GameLog,
  GameStarted,
  GameExit,
  ThemeConfig,
  ImportOutcome,
} from "./types";

// tauri-specta 生成的命令返回 { status:"ok",data } | { status:"error",error }(不抛错)。
type SpectaResult<T, E> = { status: "ok"; data: T } | { status: "error"; error: E };

// 把「返回 Result 的命令」类型映射成「成功直接 resolve、失败 throw」的旧签名。
type Unwrap<F> = F extends (...args: infer A) => Promise<SpectaResult<infer T, unknown>>
  ? (...args: A) => Promise<T>
  : F;

// 解包:error → throw(还原 try/catch 约定),ok → data。
async function unwrap<T>(p: Promise<SpectaResult<T, unknown>>): Promise<T> {
  const r = await p;
  if (r.status === "error") throw r.error;
  return r.data;
}

/** export_modpack 的对象入参(9 个位置参数太长,这里收敛成对象更顺手)。 */
export interface ExportModpackOpts {
  root: string;
  instanceId: string;
  target: string;
  dest: string | null;
  packName: string;
  packVersion?: string | null;
  mcVersion: string;
  loader: string | null;
  loaderVersion: string | null;
}

// 少数命令保留更顺手的调用形态(对象入参 / 前端主题类型);其余命令由 commands 自动透传。
const overrides = {
  exportModpack: (o: ExportModpackOpts): Promise<string> =>
    unwrap(
      commands.exportModpack(
        o.root,
        o.instanceId,
        o.target,
        o.dest,
        o.packName,
        o.packVersion ?? null,
        o.mcVersion,
        o.loader,
        o.loaderVersion,
      ),
    ),
  // install_modpack:provider 感知的整合包安装(Modrinth / CurseForge,按 version_id 取档安装)。
  // bindings 重新生成前 commands.installModpack 可能尚不存在,故缺省回退到原始 invoke;
  // 生成后两条路径同名同参,行为一致,可安全保留本 override。
  installModpack: (
    root: string,
    provider: string | null,
    project: string,
    versionId: string,
    name: string | null,
  ): Promise<ImportOutcome> => {
    const gen = (commands as Record<string, unknown>).installModpack;
    if (typeof gen === "function") {
      return unwrap(
        (gen as (...a: unknown[]) => Promise<SpectaResult<ImportOutcome, unknown>>)(
          root,
          provider,
          project,
          versionId,
          name,
        ),
      );
    }
    return rawInvoke<ImportOutcome>("install_modpack", { root, provider, project, versionId, name });
  },
  // 主题:对前端暴露 theme.ts 的严格 ThemeConfig(后端 wire 形状更宽松,边界处转换)。
  getTheme: (): Promise<ThemeConfig> => unwrap(commands.getTheme()) as Promise<ThemeConfig>,
  setTheme: (cfg: ThemeConfig): Promise<null> =>
    unwrap(commands.setTheme(cfg as unknown as Parameters<typeof commands.setTheme>[0])),
};

type Api = Omit<{ [K in keyof typeof commands]: Unwrap<(typeof commands)[K]> }, keyof typeof overrides> &
  typeof overrides;

/**
 * 命令调用层。签名/类型由 Rust(bindings.ts)生成;本代理负责解包 Result + 应用上面的 overrides。
 * 例:`await api.modrinthSearch(q, "mod", mc, loader, null, null, "modrinth", "relevance", null)` —— 成功得数组,失败抛错。
 */
export const api: Api = new Proxy({} as Api, {
  get(_t, key: string) {
    if (key in overrides) return (overrides as Record<string, unknown>)[key];
    const fn = (commands as Record<string, unknown>)[key];
    if (typeof fn !== "function") return fn;
    return (...args: unknown[]) =>
      unwrap((fn as (...a: unknown[]) => Promise<SpectaResult<unknown, unknown>>)(...args));
  },
});

// ===== 事件订阅封装 =====
// 每个 onXxx 返回一个同步可调用的取消函数;内部在 listener 就绪后再真正解绑,
// 调用方在 onCleanup 里直接调用即可,无需 await。

/** 把异步 listen 包装成同步可取消的订阅 */
function subscribe<T>(event: string, cb: (payload: T) => void): () => void {
  let unlisten: UnlistenFn | null = null;
  let cancelled = false;

  listen<T>(event, (e) => cb(e.payload))
    .then((fn) => {
      // 若在 listener 就绪前就已取消,则立即解绑
      if (cancelled) fn();
      else unlisten = fn;
    })
    .catch((err) => {
      // 监听注册失败不应让 UI 崩溃,仅记录
      console.error(`[ipc] 订阅事件 ${event} 失败:`, err);
    });

  return () => {
    cancelled = true;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  };
}

/** 订阅安装进度,返回 unlisten */
export function onInstallProgress(cb: (p: InstallProgress) => void): () => void {
  return subscribe<InstallProgress>("install://progress", cb);
}

/** 订阅启动进度,返回 unlisten */
export function onLaunchProgress(cb: (p: LaunchProgress) => void): () => void {
  return subscribe<LaunchProgress>("launch://progress", cb);
}

/** 订阅游戏日志,返回 unlisten */
export function onGameLog(cb: (log: GameLog) => void): () => void {
  return subscribe<GameLog>("game://log", cb);
}

/** 订阅「进程已真正启动」事件,返回 unlisten */
export function onGameStarted(cb: (e: GameStarted) => void): () => void {
  return subscribe<GameStarted>("game://started", cb);
}

/** 订阅「进程已退出」事件(含崩溃原因/建议),返回 unlisten */
export function onGameExit(cb: (e: GameExit) => void): () => void {
  return subscribe<GameExit>("game://exit", cb);
}
