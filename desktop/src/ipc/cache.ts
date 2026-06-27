// 进程内只读请求缓存:Discover 的搜索结果 / 项目详情 / 版本列表等目录数据按 key 记忆,
// 切换内容类型再切回、进出详情页时命中缓存、不重复请求。失败不缓存(允许重试);
// 数据按会话生命周期缓存(目录变化很慢),刷新/重启应用即清空。
const store = new Map<string, Promise<unknown>>();

/**
 * 按 `key` 记忆一次异步请求:已有则直接返回(去重并发 + 复用结果),否则执行 `fetcher`
 * 并缓存其 promise;`fetcher` 抛错时驱逐该 key,使下次可重试。
 */
export function cached<T>(key: string, fetcher: () => Promise<T>): Promise<T> {
  const hit = store.get(key);
  if (hit) return hit as Promise<T>;
  const p = fetcher().catch((e) => {
    store.delete(key);
    throw e;
  });
  store.set(key, p);
  return p;
}

/** 失效单个 key(数据被本地修改后调用,使下次读取重新拉取最新值)。 */
export function invalidate(key: string): void {
  store.delete(key);
}

/** 清空全部缓存(手动刷新目录时调用)。 */
export function clearCache(): void {
  store.clear();
}
