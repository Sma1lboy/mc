import { api } from "../ipc/api";
import { cached } from "../ipc/cache";
import type { SearchHit } from "../ipc/types";

/** Discover 搜索每页条数。预加载与 ContentBrowser 共用,缓存 key 才对得上。 */
export const SEARCH_PAGE = 30;

/** 多选 facet(仅 Modrinth 消费);与后端 SearchFacetsArg 同形。 */
export interface ContentSearchFacets {
  categories: string[];
  loaders: string[];
  game_versions: string[];
  environment: string | null;
  open_source: boolean;
}

export interface ContentSearchArgs {
  provider: string;
  kind: string;
  mcVersion: string;
  loader: string | null;
  query: string;
  sort: string;
  facets: ContentSearchFacets | null;
  offset: number;
}

function searchKey(a: ContentSearchArgs): string {
  return ["search", a.provider, a.kind, a.mcVersion || "", a.loader ?? "", a.query, a.sort, JSON.stringify(a.facets), a.offset].join("|");
}

/**
 * 会话缓存的内容搜索。ContentBrowser(实际浏览)与 Discover 预加载共用同一 key:
 * 同一(平台/类型/版本/loader/关键词/排序/facet/页)命中缓存即零网络。
 */
export function searchContent(a: ContentSearchArgs): Promise<SearchHit[]> {
  return cached(searchKey(a), () =>
    api.modrinthSearch(a.query, a.kind, a.mcVersion || null, a.loader, SEARCH_PAGE, a.offset, a.provider, a.sort, a.facets),
  );
}

/**
 * 预加载:后台预取若干类型的「默认首屏」(空关键词 / 无 facet / Modrinth / 第一页),
 * 切到该类型时直接命中缓存、即时显示。失败静默(纯预热)。
 */
export function prefetchKinds(kinds: string[], provider = "modrinth"): void {
  for (const kind of kinds) {
    void searchContent({ provider, kind, mcVersion: "", loader: null, query: "", sort: "relevance", facets: null, offset: 0 }).catch(() => {});
  }
}
