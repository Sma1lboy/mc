import { t } from "../i18n";

/**
 * 一个实例 loader 实际能加载的 mod loader 集合(与后端 `modplatform::accepted_loaders` 对齐)。
 *
 * Quilt 向后兼容 Fabric mod,故 quilt 实例同时接受 `fabric` —— 否则 UI 会把大量只发布
 * fabric 版本的 mod 误判为「不兼容」。其余 loader 只接受自身。空串返回空集。
 */
export function acceptedLoaders(loader: string): string[] {
  const l = loader.trim().toLowerCase();
  if (!l) return [];
  return l === "quilt" ? ["quilt", "fabric"] : [l];
}

/** loader 规范显示名(原版用本地化文案,其余用品牌大小写如 NeoForge)。 */
const LOADER_NAMES = (): Record<string, string> => ({
  vanilla: t("store.loader.vanilla"),
  forge: "Forge",
  neoforge: "NeoForge",
  fabric: "Fabric",
  quilt: "Quilt",
});

/**
 * loader 的显示名。各处(实例行/详情/经典启动)统一走这里,避免有的 naive 首字母大写
 * 把 neoforge 显示成 "Neoforge"、vanilla 显示成英文。未知值首字母大写兜底,空串返回空。
 */
export function loaderLabel(loader: string): string {
  const l = (loader || "").trim().toLowerCase();
  if (!l) return "";
  return LOADER_NAMES()[l] ?? l.charAt(0).toUpperCase() + l.slice(1);
}

import type { InstanceSummary, ModrinthVersion, ProjectKind } from "../ipc/types";

/** 可装进实例的内容类型(整合包走单独的安装流程)。 */
export type InstallableKind = Exclude<ProjectKind, "modpack">;

/** 一个平台版本是否兼容实例(游戏版本必须命中;mod 还要求 loader 兼容)。 */
export function versionMatches(version: ModrinthVersion, inst: InstanceSummary, kind: InstallableKind): boolean {
  if (!version.game_versions.includes(inst.mc_version)) return false;
  if (kind !== "mod") return true;
  if (inst.loader === "vanilla") return false;
  // Quilt 实例也接受 fabric 版本。
  return acceptedLoaders(inst.loader).some((l) => version.loaders.includes(l));
}

export function compatibleVersionsFor(
  versions: ModrinthVersion[],
  inst: InstanceSummary,
  kind: InstallableKind,
): ModrinthVersion[] {
  return versions.filter((version) => versionMatches(version, inst, kind));
}
