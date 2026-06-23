import { Component, createEffect, createSignal, For, Show } from "solid-js";
import { ModpackListItem, type ModpackHit } from "./ModpackListItem";
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
    id: h.project_id,
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
  /** 点击某行的「添加/下载」按钮:传入命中项(hit.id 即 project_id)。 */
  onAdd: (hit: ModpackHit) => void;
  /** 正在安装的 project_id(= hit.id);用于把该行按钮置为「安装中…」并禁用全部按钮。 */
  adding?: string | null;
  /** 点击行主体(非按钮)时打开详情;缺省则整行点击等同 onAdd。 */
  onOpenDetail?: (hit: ModpackHit) => void;
  /** 自定义搜索框占位文案。 */
  placeholder?: string;
  /** 某行按钮在禁用时的悬停提示(如数据包未选存档)。返回非空串则该行禁用并展示该提示。 */
  disabledReason?: (hit: ModpackHit) => string | null;
}

const ADD_BTN =
  "shrink-0 h-[28px] px-[12px] rounded-ctl bg-a-4 text-white text-[12px] font-semibold cursor-pointer " +
  "transition-opacity duration-[var(--dur)] ease-app hover:opacity-90 disabled:opacity-50 disabled:cursor-default";

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
        toast({ type: "error", message: `搜索失败:${e}` });
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
    setLoading(true);
    setReachedEnd(false);
    void fetchPage(q, 0).then((hits) => {
      setResults(hits ?? []);
      setLoading(false);
    });
  });

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
        placeholder={props.placeholder ?? "搜索 Modrinth…"}
      />

      <Show when={!loading()} fallback={<div class="flex justify-center p-[28px]"><Spinner /></div>}>
        <Show
          when={results().length > 0}
          fallback={
            <div class="p-[24px] text-dim text-center text-[13px]">
              <Show
                when={!backendUnavailable()}
                fallback={"浏览器预览未连接桌面后端。打开桌面应用后即可搜索 Modrinth。"}
              >
                {debounced().trim() ? "没有结果,换个关键词试试。" : "输入关键词搜索 Modrinth。"}
              </Show>
            </div>
          }
        >
          <div class="flex flex-col gap-[8px]">
            <For each={results()}>
              {(raw) => {
                const hit = toHit(raw);
                const reason = () => props.disabledReason?.(hit) ?? null;
                const disabled = () => props.adding != null || reason() != null;
                return (
                  <ModpackListItem
                    hit={hit}
                    onClick={props.onOpenDetail ?? props.onAdd}
                    action={
                      <button
                        class={ADD_BTN}
                        disabled={disabled()}
                        title={reason() ?? ""}
                        onClick={() => props.onAdd(hit)}
                      >
                        {props.adding === hit.id ? "安装中…" : "添加"}
                      </button>
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
