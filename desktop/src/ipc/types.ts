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

/** Modrinth 搜索可检索的资源类型 */
export type ProjectKind = "mod" | "modpack" | "shader" | "resourcepack";

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
