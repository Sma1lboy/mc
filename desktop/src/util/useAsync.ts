import { useCallback, useEffect, useRef, useState } from "react";

/**
 * createResource 的 React 替身。把 Solid 的
 *   const [data] = createResource(() => src(), (s) => fetch(s));
 *   ...data()          // 值(未就绪为 undefined)
 *   ...data.loading    // 是否在拉
 * 映射为:
 *   const { data, loading, error, refetch } = useAsync(() => fetch(src), [src]);
 *
 * deps 变化即重拉(等价 Solid resource 的 source 变化)。竞态用一次性 id 守卫:
 * 只让「最新一次」的结果落地,旧的丢弃(切 deps 快时不会用到过期数据)。
 * refetch 手动重拉(等价 resource 的 refetch),用当前闭包里的 fetcher。
 */
export function useAsync<T>(
  fetcher: () => Promise<T>,
  deps: readonly unknown[],
): { data: T | undefined; loading: boolean; error: unknown; refetch: () => void } {
  const [data, setData] = useState<T | undefined>(undefined);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<unknown>(undefined);
  // 递增 id:每次发起 +1,回调只在 id 仍是最新时落地(丢弃过期响应)。
  const runId = useRef(0);
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;

  const run = useCallback(() => {
    const id = ++runId.current;
    setLoading(true);
    setError(undefined);
    fetcherRef.current().then(
      (v) => {
        if (id !== runId.current) return; // 被更晚的一次覆盖,丢弃
        setData(v);
        setLoading(false);
      },
      (e) => {
        if (id !== runId.current) return;
        setError(e);
        setLoading(false);
      },
    );
  }, []);

  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(run, deps);

  return { data, loading, error, refetch: run };
}
