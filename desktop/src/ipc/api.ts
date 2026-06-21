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
  ModrinthProject,
  GlobalSettings,
  InstanceConfig,
  ModInfo,
  InstallReport,
  PackKind,
  PackInfo,
  WorldInfo,
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

  /** 取实例的游戏目录绝对路径(用于「打开游戏目录」) */
  instanceDir(root: string, id: string): Promise<string> {
    return invoke<string>("instance_dir", { root, id });
  },

  /** 读取单实例配置(名字/内存/Java/JVM/窗口…) */
  getInstanceConfig(root: string, id: string): Promise<InstanceConfig> {
    return invoke<InstanceConfig>("get_instance_config", { root, id });
  },

  /** 写单实例配置(持久化到该实例的 instance.json) */
  setInstanceConfig(root: string, id: string, config: InstanceConfig): Promise<void> {
    return invoke<void>("set_instance_config", { root, id, config });
  },

  /** 把本地图片设为实例图标(拷贝到 versions/<id>/icon.png);之后 list 会探测回传 */
  setInstanceIcon(root: string, id: string, source: string): Promise<void> {
    return invoke<void>("set_instance_icon", { root, id, source });
  },

  /** 列出某实例的本地 mod(含启停态) */
  instanceMods(root: string, id: string): Promise<ModInfo[]> {
    return invoke<ModInfo[]>("instance_mods", { root, id });
  },

  /** 启用/停用一个 mod(.jar ↔ .jar.disabled) */
  setModEnabled(root: string, id: string, fileName: string, enabled: boolean): Promise<void> {
    return invoke<void>("set_mod_enabled", { root, id, fileName, enabled });
  },

  /** 删除一个 mod 文件 */
  deleteMod(root: string, id: string, fileName: string): Promise<void> {
    return invoke<void>("delete_mod", { root, id, fileName });
  },

  /** 从 Modrinth 把一个 mod(+必需依赖)装进实例 */
  installMod(
    root: string,
    id: string,
    project: string,
    mcVersion: string,
    loader: string,
  ): Promise<InstallReport> {
    return invoke<InstallReport>("install_mod", { root, id, project, mcVersion, loader });
  },

  /** 列出某实例下指定类型的包(资源包 / 光影 / 数据包),含启停态 */
  instancePacks(root: string, id: string, kind: PackKind): Promise<PackInfo[]> {
    return invoke<PackInfo[]>("instance_packs", { root, id, kind });
  },

  /** 启用/停用一个包(.zip ↔ .zip.disabled) */
  setPackEnabled(
    root: string,
    id: string,
    kind: PackKind,
    fileName: string,
    enabled: boolean,
  ): Promise<void> {
    return invoke<void>("set_pack_enabled", { root, id, kind, fileName, enabled });
  },

  /** 删除一个包(移入回收站,可找回) */
  deletePack(root: string, id: string, kind: PackKind, fileName: string): Promise<void> {
    return invoke<void>("delete_pack", { root, id, kind, fileName });
  },

  /** 从 Modrinth 安装一个包到实例对应目录,返回落盘文件名 */
  installPack(
    root: string,
    id: string,
    kind: PackKind,
    project: string,
    mcVersion: string,
  ): Promise<string> {
    return invoke<string>("install_pack", { root, id, kind, project, mcVersion });
  },

  /** 列出某实例的存档世界 */
  instanceWorlds(root: string, id: string): Promise<WorldInfo[]> {
    return invoke<WorldInfo[]>("instance_worlds", { root, id });
  },

  /** 删除一个存档世界(移入回收站,可找回) */
  deleteWorld(root: string, id: string, folder: string): Promise<void> {
    return invoke<void>("delete_world", { root, id, folder });
  },

  /** 把一个存档打成 zip 备份到 destDir,返回写出的 zip 绝对路径 */
  backupWorld(root: string, id: string, folder: string, destDir: string): Promise<string> {
    return invoke<string>("backup_world", { root, id, folder, destDir });
  },

  /** 重命名存档的显示名(改 level.dat 的 LevelName,不改文件夹名) */
  renameWorld(root: string, id: string, folder: string, newName: string): Promise<void> {
    return invoke<void>("rename_world", { root, id, folder, newName });
  },

  /** 删除实例(移除整个版本目录,含 mods/saves;破坏性,调用方需先确认) */
  deleteInstance(root: string, id: string): Promise<void> {
    return invoke<void>("delete_instance", { root, id });
  },

  /**
   * 从零创建实例(装核心 + 命名实例);进度走 install://progress,返回新实例 id。
   * loader: "vanilla" | "fabric" | "quilt" | "forge" | "neoforge"。
   * forge/neoforge 需要 loaderVersion(forge build / neoforge 版本)。
   */
  createInstance(
    root: string,
    name: string,
    mcVersion: string,
    loader: string,
    loaderVersion?: string | null,
  ): Promise<string> {
    return invoke<string>("create_instance", {
      root,
      name,
      mcVersion,
      loader,
      loaderVersion: loaderVersion ?? null,
    });
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

  /** 取一个整合包项目的完整详情(简介标签页用:长描述/画廊/外部链接) */
  modrinthProject(projectId: string): Promise<ModrinthProject> {
    return invoke<ModrinthProject>("modrinth_project", { projectId });
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

  /** 读取全局设置(下载源/并发/默认内存/Java…) */
  getSettings(): Promise<GlobalSettings> {
    return invoke<GlobalSettings>("get_settings");
  },

  /** 持久化全局设置 */
  setSettings(settings: GlobalSettings): Promise<void> {
    return invoke<void>("set_settings", { settings });
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
