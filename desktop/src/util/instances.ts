/** 纯实例工具:与副作用无关的小函数(排序/筛选等),Home / Library / Rail 共用。 */

/**
 * 实例按「上次游玩」降序(最近在前;未玩过=0 沉底)。返回新数组,不改原数组。
 * Home「继续游玩」、Library 列表、Rail 最近实例统一走这里,避免三处各写一份排序。
 */
export function sortByRecent<T extends { last_played?: number | null }>(list: readonly T[]): T[] {
  return [...list].sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0));
}
