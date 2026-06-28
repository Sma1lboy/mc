// IPC DTO 类型:由 Rust 经 tauri-specta 生成到 ./bindings.ts(单一真相,改 Rust 即重生成),
// 这里只做「再导出 + 历史命名别名」,杜绝手写 TS 与后端漂移。
// 命令面之外的类型(事件 payload、纯前端联合)不在命令签名里,故仍在本文件手写。

export type {
  AccountKind,
  AccountSummary,
  GameRoot,
  GlobalSettings,
  InstanceSource,
  InstanceSummary,
  ManifestVersion,
  ModInfo,
  ModUpdate,
  PackKind,
  ReleaseKind,
  RootKind,
  ScreenshotInfo,
  SearchHit,
  WorldInfo,
  ModInstallReport,
  InstalledMod,
  VersionInstallReport,
  GalleryImage,
  FacetTagsDto,
  CategoryTag,
  LoaderTag,
  GameVersionTag,
  ProfileSkins,
  SkinInfo,
  CapeInfo,
  ServerStatus,
} from "./bindings";

// serde 序列化/反序列化形状不同 → 取「读取(反序列化)」形状(字段齐全)给编辑器用。
export type { InstanceConfig_Deserialize as InstanceConfig, PackInfo_Deserialize as PackInfo } from "./bindings";

// ThemeConfig 是前端主题引擎类型(严格联合 + 数值),由 theme/theme.ts 拥有;命令边界做宽松适配。
export type { ThemeConfig } from "../theme/theme";

// 名称对齐:Rust 结构名 → 前端历史用名。
export type { LoaderKind as Loader } from "./bindings";
export type { VersionDetail as ModrinthVersion } from "./bindings";
export type { ProjectDetail as ModrinthProject } from "./bindings";
export type { DeviceCodeDto as DeviceCode } from "./bindings";
export type { JavaDto as JavaInstall } from "./bindings";
export type { ImportOutcomeDto as ImportOutcome } from "./bindings";
export type { BlockedFileDto as BlockedFile } from "./bindings";

// ===== 命令面之外的类型(tauri-specta 不收集,保持手写)=====

/** Modrinth 项目类型(搜索/分类用;Rust 命令以字符串接收)。 */
export type ProjectKind = "mod" | "modpack" | "shader" | "resourcepack" | "datapack";

/** 主题明暗模式(前端联合;ThemeConfig.mode 用)。 */
export type ThemeMode = "dark" | "light" | "system";

// ===== 事件 payload(由 Tauri event 推送)=====
// 类型同样由 Rust 生成(lib.rs 用 .typ::<…>() 注册进 bindings);emit/listen 机制不变。
// 安装/启动进度都用 mc_types::Progress(含 stage/current/total/speed_bps)。
export type { GameLog, GameStarted, GameExit } from "./bindings";
export type { Progress as InstallProgress, Progress as LaunchProgress } from "./bindings";
