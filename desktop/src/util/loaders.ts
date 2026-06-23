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

/** loader 规范显示名(原版用中文,其余用品牌大小写如 NeoForge)。 */
const LOADER_NAMES: Record<string, string> = {
  vanilla: "原版",
  forge: "Forge",
  neoforge: "NeoForge",
  fabric: "Fabric",
  quilt: "Quilt",
};

/**
 * loader 的显示名。各处(实例行/详情/经典启动)统一走这里,避免有的 naive 首字母大写
 * 把 neoforge 显示成 "Neoforge"、vanilla 显示成英文。未知值首字母大写兜底,空串返回空。
 */
export function loaderLabel(loader: string): string {
  const l = (loader || "").trim().toLowerCase();
  if (!l) return "";
  return LOADER_NAMES[l] ?? l.charAt(0).toUpperCase() + l.slice(1);
}
