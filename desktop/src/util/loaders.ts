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
