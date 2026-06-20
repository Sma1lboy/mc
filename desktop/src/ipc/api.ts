// IPC 薄封装层
// ------------------------------------------------------------------
// 把 @tauri-apps/api 的 invoke / listen 收敛到一个 `api` 对象 + 一组
// onXxx 事件订阅函数。页面只依赖这一层,不直接散落 invoke 字符串,便于
// 统一改命名、加日志、做 mock。每个方法都是强类型的,泛型参数对应
// types.ts 里的 DTO。
// ------------------------------------------------------------------

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  GameRoot,
  InstanceSummary,
  ManifestVersion,
  AccountSummary,
  DeviceCode,
  JavaInstall,
  SearchHit,
  ThemeConfig,
  ProjectKind,
  InstallProgress,
  LaunchProgress,
  GameLog,
  ImportOutcome,
  ModrinthVersion,
} from "./types";

// ===== 命令封装 =====
// 注意:Tauri 会把 snake_case 命令名映射到 Rust 端 #[tauri::command]。
// 参数对象的 key 必须与 Rust 函数参数名一致(camelCase ↔ snake_case 由
// Tauri 自动转换,这里统一用与后端约定一致的 camelCase)。

export const api = {
  /** 列出所有已发现的游戏根目录 */
  listRoots(): Promise<GameRoot[]> {
    return invoke<GameRoot[]>("list_roots");
  },

  /** 列出某个根目录下的实例 */
  listInstances(root: string): Promise<InstanceSummary[]> {
    return invoke<InstanceSummary[]>("list_instances", { root });
  },

  /** 列出官方版本清单;snapshot=true 时包含快照 */
  listVersions(snapshot: boolean): Promise<ManifestVersion[]> {
    return invoke<ManifestVersion[]>("list_versions", { snapshot });
  },

  /** 列出本地账号 */
  listAccounts(): Promise<AccountSummary[]> {
    return invoke<AccountSummary[]>("list_accounts");
  },

  /** 微软登录①:启动设备码流,返回 user_code/验证地址 + 轮询用 device_code */
  msaLoginStart(): Promise<DeviceCode> {
    return invoke<DeviceCode>("msa_login_start");
  },

  /** 微软登录②:阻塞轮询直到用户在浏览器完成,走完认证链并落库,返回新账号 */
  msaLoginPoll(deviceCode: string, interval: number): Promise<AccountSummary> {
    return invoke<AccountSummary>("msa_login_poll", { deviceCode, interval });
  },

  /** 添加离线账号(用户名 → 稳定 UUID),并设为当前账号 */
  addOfflineAccount(name: string): Promise<AccountSummary> {
    return invoke<AccountSummary>("add_offline_account", { name });
  },

  /** 切换当前账号 */
  selectAccount(uuid: string): Promise<void> {
    return invoke<void>("select_account", { uuid });
  },

  /** 移除账号 */
  removeAccount(uuid: string): Promise<void> {
    return invoke<void>("remove_account", { uuid });
  },

  /** 检测系统中可用的 Java */
  detectJava(): Promise<JavaInstall[]> {
    return invoke<JavaInstall[]>("detect_java");
  },

  /** 安装某个官方版本到指定根目录;进度通过 install://progress 事件推送 */
  installVersion(root: string, id: string): Promise<void> {
    return invoke<void>("install_version", { root, id });
  },

  /**
   * 启动一个实例。进度走 launch://progress,日志走 game://log。
   * @param online true=正版/在线登录,false=离线
   */
  launchInstance(
    root: string,
    id: string,
    name: string,
    online: boolean,
  ): Promise<void> {
    return invoke<void>("launch_instance", { root, id, name, online });
  },

  /** Modrinth 搜索。gameVersion / loader 为 null 表示不限。 */
  modrinthSearch(
    query: string,
    kind: ProjectKind,
    gameVersion: string | null,
    loader: string | null,
  ): Promise<SearchHit[]> {
    // 后端命令签名:modrinth_search(query, kind, game_version, loader)
    return invoke<SearchHit[]>("modrinth_search", {
      query,
      kind,
      gameVersion,
      loader,
    });
  },

  /**
   * 导入一个整合包(.mrpack / CurseForge zip / MultiMC / MCBBS,自动识别格式)。
   * 返回建好的实例 id + 需手动下载的 CurseForge blocked 文件。
   */
  importModpack(
    root: string,
    path: string,
    instanceId: string | null,
  ): Promise<ImportOutcome> {
    return invoke<ImportOutcome>("import_modpack", { root, path, instanceId });
  },

  /**
   * 把实例导出为整合包。target = "modrinth" | "curseforge" |
   * "modlist[:md|json|csv|txt|html]"。返回写出的文件路径。
   */
  exportModpack(opts: {
    root: string;
    instanceId: string;
    target: string;
    dest?: string | null;
    packName: string;
    packVersion?: string | null;
    mcVersion: string;
    loader?: string | null;
    loaderVersion?: string | null;
  }): Promise<string> {
    return invoke<string>("export_modpack", {
      root: opts.root,
      instanceId: opts.instanceId,
      target: opts.target,
      dest: opts.dest ?? null,
      packName: opts.packName,
      packVersion: opts.packVersion ?? null,
      mcVersion: opts.mcVersion,
      loader: opts.loader ?? null,
      loaderVersion: opts.loaderVersion ?? null,
    });
  },

  /**
   * 从 Modrinth 安装一个整合包(取最新版 .mrpack → 下载 + 安装成可启动实例)。
   * 首次会下载原版 Minecraft 与依赖,可能耗时数分钟。
   */
  installModrinthModpack(
    root: string,
    projectId: string,
    instanceId?: string | null,
  ): Promise<ImportOutcome> {
    return invoke<ImportOutcome>("install_modrinth_modpack", {
      root,
      projectId,
      instanceId: instanceId ?? null,
    });
  },

  /** 列出一个整合包项目的所有版本详情(详情页用) */
  modrinthVersions(projectId: string): Promise<ModrinthVersion[]> {
    return invoke<ModrinthVersion[]>("modrinth_versions", { projectId });
  },

  /** 从指定 .mrpack 直链安装(详情页「安装此版本」) */
  installModpackUrl(
    root: string,
    url: string,
    instanceId?: string | null,
  ): Promise<ImportOutcome> {
    return invoke<ImportOutcome>("install_modpack_url", {
      root,
      url,
      instanceId: instanceId ?? null,
    });
  },

  /** 读取持久化的主题配置 */
  getTheme(): Promise<ThemeConfig> {
    return invoke<ThemeConfig>("get_theme");
  },

  /** 持久化主题配置 */
  setTheme(cfg: ThemeConfig): Promise<void> {
    return invoke<void>("set_theme", { cfg });
  },
};

// ===== 事件订阅封装 =====
// 每个 onXxx 返回一个 Promise<UnlistenFn>;由于 listen 本身是异步的,
// 这里返回一个「同步可调用」的取消函数,内部在 listener 就绪后再真正解绑。
// 这样调用方在 onCleanup 里直接调用返回值即可,无需 await。

/** 把异步 listen 包装成同步可取消的订阅 */
function subscribe<T>(
  event: string,
  cb: (payload: T) => void,
): () => void {
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
export function onInstallProgress(
  cb: (p: InstallProgress) => void,
): () => void {
  return subscribe<InstallProgress>("install://progress", cb);
}

/** 订阅启动进度,返回 unlisten */
export function onLaunchProgress(
  cb: (p: LaunchProgress) => void,
): () => void {
  return subscribe<LaunchProgress>("launch://progress", cb);
}

/** 订阅游戏日志,返回 unlisten */
export function onGameLog(cb: (log: GameLog) => void): () => void {
  return subscribe<GameLog>("game://log", cb);
}
