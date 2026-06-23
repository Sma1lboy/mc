import { Component, createEffect, createSignal, For, Show } from "solid-js";
import { ModpackListItem } from "./ModpackListItem";
import type { ModpackHit } from "./ModpackCard";
import { ACCENT_BTN_COMPACT } from "./styles";
import { SearchBox } from "./SearchBox";
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import type { ProjectKind, SearchHit } from "../ipc/types";

/**
 * ContentBrowser —— 复用 Discover 的搜索体验(SearchBox + 防抖 + 分页 +
 * <ModpackListItem> 列表 + 「加载更多」),供 Discover 页与实例管理弹窗(Mods /
 * 资源包 / 光影 / 数据包)共用。
 *
 * 与 Discover 不同处:把 mcVersion + loader 透传给 modrinthSearch,使结果按该实例
 * 过滤;每行带一个尾部「添加/下载」按钮,点击回调 onAdd(由调用方决定打开详情还是
 * 直接安装最新兼容版)。
 */

const PAGE = 30;

/** SearchHit → ModpackHit(列表项契约)。 */
function toHit(h: SearchHit): ModpackHit {
  return {
    id: h.id,
    slug: h.slug,
    title: h.title,
    description: h.description,
    author: h.author,
    downloads: h.downloads,
    icon_url: h.icon_url || undefined,
    gallery_url: h.gallery_url || undefined,
    categories: h.categories,
  };
}

function isDesktopBackendUnavailable(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes("__TAURI_INTERNALS__") ||
    message.includes("reading 'invoke'") ||
    message.includes("Cannot read properties of undefined")
  );
}

export interface ContentBrowserProps {
  /** 搜索的项目类型(mod / resourcepack / shader / datapack / modpack)。 */
  kind: ProjectKind;
  /** 实例的 MC 版本,用于过滤兼容结果。 */
  mcVersion: string;
  /** 加载器(fabric/forge/…);资源包/光影/数据包不细分加载器,传 null。 */
  loader: string | null;
  /** 点击某行的「添加/下载」按钮:传入命中项(hit.id 即 project_id)。缺省则不渲染该按钮,点击行进详情。 */
  onAdd?: (hit: ModpackHit) => void;
  /** 紧凑模式:结果区限高内滚,避免在标签页里把下方区块(已安装等)顶没。 */
  compact?: boolean;
  /** 正在安装的 project_id 集合(= hit.id);只把这些行置「安装中…」并禁用,其它行照常可点(后台并行)。 */
  addingIds?: Set<string>;
  /** 点击行主体(非按钮)时打开详情;缺省则整行点击等同 onAdd。 */
  onOpenDetail?: (hit: ModpackHit) => void;
  /** 自定义搜索框占位文案。 */
  placeholder?: string;
  /** 某行按钮在禁用时的悬停提示(如数据包未选存档)。返回非空串则该行禁用并展示该提示。 */
  disabledReason?: (hit: ModpackHit) => string | null;
  /** 挂载即聚焦搜索框(进入浏览/添加模式时直接可打字)。 */
  autofocus?: boolean;
  /** 搜索框为空时按 Esc 的回调(如退出浏览返回已安装);有内容时 Esc 先清空。 */
  onEscape?: () => void;
  /** 本次浏览已添加的 project_id 集合:这些行按钮显示「已添加」并禁用,给即时反馈。 */
  addedIds?: Set<string>;
}

const ADD_BTN = ACCENT_BTN_COMPACT;
// 已添加:幽灵态(描边 + accent 文字),明确「装过了」且不可再点。
const ADDED_BTN =
  "shrink-0 h-[28px] px-[12px] rounded-ctl border border-glass-border bg-transparent text-a-6 text-[12px] font-semibold cursor-default";

export const ContentBrowser: Component<ContentBrowserProps> = (props) => {
  const [query, setQuery] = createSignal("");
  const [debounced, setDebounced] = createSignal("");
  const [backendUnavailable, setBackendUnavailable] = createSignal(false);
  let timer: number | undefined;

  function onInput(v: string) {
    setQuery(v);
    clearTimeout(timer);
    timer = window.setTimeout(() => setDebounced(v), 350);
  }

  const [results, setResults] = createSignal<SearchHit[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [loadingMore, setLoadingMore] = createSignal(false);
  const [reachedEnd, setReachedEnd] = createSignal(false);
  // 搜索失败(非「后端未连」)单独成态:区分「真的没结果」与「搜挂了」,后者给重试。
  const [searchError, setSearchError] = createSignal<string | null>(null);

  async function fetchPage(q: string, offset: number): Promise<SearchHit[] | null> {
    try {
      const hits = await api.modrinthSearch(
        q,
        props.kind,
        props.mcVersion || null,
        props.loader,
        PAGE,
        offset,
      );
      setReachedEnd(hits.length < PAGE);
      return hits;
    } catch (e) {
      if (isDesktopBackendUnavailable(e)) {
        setBackendUnavailable(true);
      } else {
        // 翻页失败仍 toast(已有列表在);首屏失败走 searchError 占位 + 重试。
        if (offset > 0) toast({ type: "error", message: `搜索失败:${e}` });
        else setSearchError(e instanceof Error ? e.message : String(e));
      }
      return null;
    }
  }

  // 关键词 / 类型 / 实例版本 / 加载器变化 → 重新拉第一页(替换)。
  createEffect(() => {
    const q = debounced();
    // 订阅以下信号,使切换实例/类型时也会重搜。
    void props.kind;
    void props.mcVersion;
    void props.loader;
    setBackendUnavailable(false);
    setSearchError(null);
    setLoading(true);
    setReachedEnd(false);
    void fetchPage(q, 0).then((hits) => {
      setResults(hits ?? []);
      setLoading(false);
    });
  });

  // 首屏失败后的重试:清错、重拉第一页。
  function retry() {
    setSearchError(null);
    setLoading(true);
    setReachedEnd(false);
    void fetchPage(debounced(), 0).then((hits) => {
      setResults(hits ?? []);
      setLoading(false);
    });
  }

  async function loadMore() {
    if (loadingMore() || reachedEnd()) return;
    setLoadingMore(true);
    const hits = await fetchPage(debounced(), results().length);
    if (hits) setResults((prev) => [...prev, ...hits]);
    setLoadingMore(false);
  }

  return (
    <div class="flex flex-col gap-[12px]">
      <SearchBox
        value={query()}
        onInput={onInput}
        autofocus={props.autofocus}
        onEscape={() => {
          // 有搜索词先清空;已空则上抛(退出浏览)。
          if (query()) {
            setQuery("");
            setDebounced("");
          } else {
            props.onEscape?.();
          }
        }}
        placeholder={props.placeholder ?? "搜索 Modrinth…"}
      />

      <Show when={!loading()} fallback={<div class="flex justify-center p-[28px]"><Spinner /></div>}>
        <Show
          when={results().length > 0}
          fallback={
            <Show
              when={!searchError()}
              fallback={
                <div class="flex flex-col items-center justify-center gap-[12px] py-[36px] text-center">
                  <div class="text-dim text-[13px]">搜索失败,请检查网络后重试。</div>
                  <button
                    class="h-[34px] px-[16px] rounded-ctl border border-glass-border bg-glass-card text-fg text-[13px] cursor-pointer transition-[background-color] duration-[var(--dur)] ease-app hover:bg-glass-hover"
                    onClick={retry}
                  >
                    重试
                  </button>
                </div>
              }
            >
            <div class="p-[24px] text-dim text-center text-[13px]">
              <Show
                when={!backendUnavailable()}
                fallback={"浏览器预览未连接桌面后端。打开桌面应用后即可搜索 Modrinth。"}
              >
                {debounced().trim() ? "没有结果,换个关键词试试。" : "输入关键词搜索 Modrinth。"}
              </Show>
            </div>
            </Show>
          }
        >
          <div
            class={
              "flex flex-col gap-[8px]" +
              (props.compact ? " max-h-[340px] overflow-y-auto pr-[2px]" : "")
            }
          >
            <For each={results()}>
              {(raw) => {
                const hit = toHit(raw);
                const reason = () => props.disabledReason?.(hit) ?? null;
                const added = () => props.addedIds?.has(hit.id) ?? false;
                const busy = () => props.addingIds?.has(hit.id) ?? false;
                // 只禁用「这一行」(已添加 / 该行安装中 / 该行有禁用原因);其它行后台并行不受影响。
                const disabled = () => reason() != null || added() || busy();
                const onAdd = props.onAdd;
                return (
                  <ModpackListItem
                    hit={hit}
                    onClick={props.onOpenDetail ?? onAdd ?? (() => {})}
                    action={
                      onAdd ? (
                        <button
                          class={added() ? ADDED_BTN : ADD_BTN}
                          disabled={disabled()}
                          title={reason() ?? ""}
                          onClick={() => onAdd(hit)}
                        >
                          {added() ? "已添加" : busy() ? "安装中…" : "添加"}
                        </button>
                      ) : undefined
                    }
                  />
                );
              }}
            </For>
          </div>

          <Show when={!reachedEnd()}>
            <div class="flex justify-center mt-[8px]">
              <button
                class="px-[20px] py-[8px] rounded-ctl border border-glass-border bg-glass-card text-fg text-[13px] cursor-pointer transition-[background-color] duration-[var(--dur)] ease-app hover:bg-glass-hover disabled:opacity-50 disabled:cursor-default"
                disabled={loadingMore()}
                onClick={loadMore}
              >
                {loadingMore() ? "加载中…" : "加载更多"}
              </button>
            </div>
          </Show>
        </Show>
      </Show>
    </div>
  );
};

export default ContentBrowser;
