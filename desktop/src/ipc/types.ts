// IPC 共享类型定义
// ------------------------------------------------------------------
// 这里的字段必须与后端 mc-types(ts-rs 导出)以及 Tauri command 的
// 返回结构严格一致。前端只通过这些 interface 与后端通信,做到「改字段
// 编译即报错」。后续若后端切换为 ts-rs 自动导出,可直接替换本文件。
// ------------------------------------------------------------------

/** 游戏根目录的种类(对应 docs/07 的 RootKind 枚举,后端序列化为小写字符串) */
export type RootKind = "portable" | "official" | "custom" | "default";

/** 一个 .minecraft 式游戏根目录 */
export interface GameRoot {
  /** 展示名(如「官方目录」「便携包」) */
  name: string;
  /** 绝对路径 */
  path: string;
  /** 来源种类,决定图标与排序 */
  kind: RootKind;
}

/** Mod 加载器种类 */
export type Loader = "vanilla" | "forge" | "fabric" | "quilt" | "neoforge";

/** 某个根目录下的一个可启动实例的摘要 */
export interface InstanceSummary {
  /** 实例唯一 id(通常是版本目录名) */
  id: string;
  /** 实例展示名 */
  name: string;
  /** Minecraft 版本号,如 "1.20.1" */
  mc_version: string;
  /** 加载器 */
  loader: Loader;
  /** 加载器版本(vanilla 时可为空串) */
  loader_version: string;
  /** 实例图标:可为 data URL / 本地路径 / 远程 URL,缺省时由 UI 兜底 */
  icon: string | null;
  /** 上次游玩时间(Unix 毫秒时间戳;从未游玩为 null) */
  last_played: number | null;
  /** 当前是否正在运行 */
  running: boolean;
}

/** 官方版本清单(version_manifest)里的一个版本条目 */
export interface ManifestVersion {
  /** 版本 id,如 "1.20.4" / "24w14a" */
  id: string;
  /** 版本类型 */
  kind: "release" | "snapshot" | "old_beta" | "old_alpha";
  /** 该版本 json 的下载地址 */
  url: string;
  /** version json 的 sha1 */
  sha1: string;
  /** 发布时间(ISO 8601 字符串) */
  release_time: string;
}

/** 账号种类(与 Rust AccountKind 的 serde 小写一致:microsoft 而非 msa) */
export type AccountKind = "offline" | "microsoft" | "yggdrasil";

/** 账号摘要(右侧栏账号切换器使用) */
export interface AccountSummary {
  kind: AccountKind;
  username: string;
  uuid: string;
  /** 是否为当前选中账号 */
  selected: boolean;
  /** 是否拥有正版(影响在线启动可用性) */
  owns_game: boolean;
}

/** 微软设备码登录:展示给用户的提示信息(device_code 仅用于轮询,不展示) */
export interface DeviceCode {
  user_code: string;
  verification_uri: string;
  device_code: string;
  interval: number;
  expires_in: number;
}

/** 检测到的一个 Java 安装 */
export interface JavaInstall {
  /** java 可执行文件路径 */
  path: string;
  /** 版本字符串,如 "17.0.9" */
  version: string;
  /** 是否 64 位 */
  is_64bit: boolean;
  /** 来源(如 "PATH" / "JAVA_HOME" / "下载" / 注册表) */
  source: string;
}

/** Modrinth 搜索可检索的资源类型(datapack 在 Modrinth 是带 datapack 分类的 mod 项目) */
export type ProjectKind = "mod" | "modpack" | "shader" | "resourcepack" | "datapack";

/** Modrinth 搜索命中的一个项目 */
export interface SearchHit {
  /** 项目 id(slug 或 project_id) */
  project_id: string;
  /** slug,用于打开详情页 */
  slug: string;
  /** 标题 */
  title: string;
  /** 简短描述 */
  description: string;
  /** 作者 */
  author: string;
  /** 图标 URL(可为空) */
  icon_url: string | null;
  /** 封面/画廊大图 URL(ModpackCard 大卡使用,可为空) */
  gallery_url: string | null;
  /** 下载数 */
  downloads: number;
  /** 收藏/关注数 */
  follows: number;
  /** 分类标签 */
  categories: string[];
  /** 项目类型 */
  project_type: ProjectKind;
}

/** 主题模式('system' 跟随系统 prefers-color-scheme) */
export type ThemeMode = "dark" | "light" | "system";

/** 主题配置(后端持久化 + 前端注入色阶) */
export interface ThemeConfig {
  /** 深 / 浅模式 */
  mode: ThemeMode;
  /** accent 色相 0-360 */
  hue: number;
  /** accent 饱和度 0-100 */
  saturation: number;
  /** accent 明度 0-100 */
  lightness: number;
}

/** 全局启动器设置(跨实例;对应后端 GlobalSettings,持久化到 settings.json) */
export interface GlobalSettings {
  /** 下载源:"official"(官方直连)或 "bmclapi"(国内镜像) */
  download_source: string;
  /** 下载并发数 */
  concurrency: number;
  /** 新建实例的默认堆内存(MiB) */
  default_memory_mb: number;
  /** 全局 Java 可执行文件路径;null = 自动检测 */
  java_path: string | null;
  /** 是否强制走镜像(与 download_source 任一指向镜像即生效) */
  use_mirror: boolean;
  /** 界面语言(如 zh-CN / en-US) */
  language: string;
  /** 可选远端服务地址 */
  server_url: string | null;
  /** 额外自定义数据根目录 */
  custom_roots: string[];
}

/** 单实例配置(对应后端 InstanceConfig,持久化到该实例的 instance.json) */
export interface InstanceConfig {
  /** 展示名;null = 用 id */
  name: string | null;
  /** 最大堆内存(MiB) */
  memory_mb: number;
  /** 该实例的 Java 路径;null = 跟随全局/自动 */
  java_path: string | null;
  /** 额外 JVM 参数 */
  jvm_args: string[];
  /** 额外游戏参数 */
  game_args: string[];
  /** 窗口宽 / 高;null = 默认 */
  width: number | null;
  height: number | null;
  fullscreen: boolean;
  /** 启动后自动加入的服务器地址 */
  server: string | null;
}

/** 实例里的一个本地 mod(对应后端 ModInfo) */
export interface ModInfo {
  /** 磁盘文件名(含 .disabled,如有);启停/删除的稳定标识 */
  file_name: string;
  enabled: boolean;
  name: string;
  version: string | null;
  mod_id: string | null;
  loader: string;
  authors: string[];
  description: string | null;
  size: number;
}

/** 包资源种类(对应后端 PackKind,serde snake_case) */
export type PackKind = "resource_pack" | "shader" | "datapack";

/** 实例里的一个本地包资源(资源包 / 光影 / 数据包;对应后端 PackInfo) */
export interface PackInfo {
  /** 磁盘文件(或目录)名,含可能的 .disabled 后缀;启停/删除的稳定标识 */
  file_name: string;
  enabled: boolean;
  /** 资源类型标签:resourcepack / shader / datapack */
  kind: string;
  /** 文件大小(字节);目录形态的数据包为 0 */
  size: number;
  /** 资源包 pack.mcmeta 描述;无则缺省 */
  description?: string;
}

/** 一张截图的元数据(对应后端 ScreenshotInfo;图片字节按需经 read_screenshot 取) */
export interface ScreenshotInfo {
  file_name: string;
  size: number;
  /** 修改时间(epoch 毫秒) */
  modified: number;
}

/** 一个存档世界(对应后端 WorldInfo) */
export interface WorldInfo {
  /** saves/ 下的目录名(backup/delete/rename 的稳定标识) */
  folder: string;
  name: string;
  /** survival / creative / adventure / spectator / unknown */
  game_mode: string;
  /** 上次游玩时间(epoch 毫秒);0 = 未知 */
  last_played: number;
  seed: number | null;
  size_bytes: number;
}

/** 一个可用的 mod 更新(对应后端 ModUpdate);字段足以直接应用更新 */
export interface ModUpdate {
  /** 当前磁盘文件名(将被替换) */
  file_name: string;
  name: string;
  /** 当前版本号(本地元数据,可能缺失) */
  current_version: string | null;
  /** 最新版本号 */
  new_version: string;
  /** 最新文件落盘名 */
  new_file_name: string;
  url: string;
  sha1: string | null;
  size: number | null;
}

/** 向实例装 mod 的结果(对应后端 InstallReport) */
export interface InstallReport {
  /** 已装入的文件 */
  installed: { project_id: string; file_name: string }[];
  /** 已满足(已存在/无需重复装)的依赖 project id */
  satisfied: string[];
  /** 找不到兼容版本、需手动处理的 required 依赖 */
  unresolved: string[];
}

/** 一个整合包版本的详情(详情页用;对应后端 VersionDetail) */
export interface ModrinthVersion {
  id: string;
  version_number: string;
  name: string;
  /** release / beta / alpha */
  version_type: string;
  game_versions: string[];
  loaders: string[];
  /** ISO 8601 发布时间 */
  date_published: string;
  downloads: number;
  /** 更新日志(markdown 原文) */
  changelog: string;
  /** 该版本 .mrpack 下载地址;无则 null(安装按钮禁用) */
  mrpack_url: string | null;
  mrpack_filename: string | null;
  file_size: number | null;
}

/** 项目画廊里的一张图(对应后端 GalleryImage) */
export interface GalleryImage {
  url: string;
  title: string | null;
  description: string | null;
  featured: boolean;
}

/** 一个整合包项目的完整详情(详情页「简介」标签页用;对应后端 ProjectDetail) */
export interface ModrinthProject {
  id: string;
  slug: string;
  title: string;
  /** 一句话简介 */
  description: string;
  /** 完整介绍正文(markdown 原文) */
  body: string;
  downloads: number;
  followers: number;
  icon_url: string | null;
  categories: string[];
  /** 画廊图片(已按 ordering 升序) */
  gallery: GalleryImage[];
  source_url: string | null;
  issues_url: string | null;
  wiki_url: string | null;
  discord_url: string | null;
}

/** 一个 CurseForge blocked 文件(作者禁第三方分发,需用户手动下载) */
export interface BlockedFile {
  name: string;
  website_url: string;
  target_dir: string;
  required: boolean;
}

/** import_modpack 的返回:建好的实例 + 需手动处理的 blocked / 跳过项 */
export interface ImportOutcome {
  instance_id: string;
  blocked: BlockedFile[];
  skipped_optional: string[];
}

// ===== 事件 payload(由 Tauri event 推送)=====

/** 安装进度事件 payload(event: "install://progress") */
export interface InstallProgress {
  /** 当前阶段描述,如「下载 libraries」 */
  stage: string;
  /** 当前已完成量 */
  current: number;
  /** 总量(为 0 时视为不确定进度) */
  total: number;
}

/** 启动进度事件 payload(event: "launch://progress") */
export interface LaunchProgress {
  stage: string;
  current: number;
  total: number;
}

/** 游戏日志事件 payload(event: "game://log") */
export interface GameLog {
  /** 一行日志文本 */
  line: string;
  /** 日志级别(后端可不提供,UI 据此着色) */
  level?: "info" | "warn" | "error";
}
