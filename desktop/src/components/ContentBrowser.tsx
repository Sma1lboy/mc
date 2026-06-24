import { Component, createEffect, createSignal, onCleanup, For, Show } from "solid-js";
import { ModpackListItem } from "./ModpackListItem";
import type { ModpackHit } from "./ModpackCard";
import { ACCENT_BTN_COMPACT } from "./styles";
import { SearchBox } from "./SearchBox";
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { t } from "../i18n";
import type { ProjectKind, SearchHit } from "../ipc/types";

/**
 * ContentBrowser —— 复用 Discover 的搜索体验(SearchBox + 防抖 + 分页 +
 * <ModpackListItem> 列表 + 「加载更多」),供 Discover 页与实例管理弹窗(Mods /
 * 资源包 / 光影 / 数据包)共用。
 *
 * 与 Discover 不同处:把 mcVersion + loader 透传给搜索命令,使结果按该实例
 * 过滤;每行带一个尾部「添加/下载」按钮,点击回调 onAdd(由调用方决定打开详情还是
 * 直接安装最新兼容版)。
 *
 * 内容平台(Modrinth / CurseForge)与排序在本组件内自管:切换即从 offset 0 重搜该
 * 平台。结果不带平台身份,故把当前选中的平台一并回传给 onAdd / onOpenDetail,
 * 调用方据此把安装路由到正确平台。CurseForge 未配置 API Key 时禁用该选项并就地提示。
 */

const PAGE = 30;

/** 内容平台标识(透传给搜索/安装命令的 provider 字符串)。 */
export type ContentProvider = "modrinth" | "curseforge";

/** 排序方式(映射后端 SortMethod)。 */
type SortKey = "relevance" | "downloads" | "updated" | "newest";

/** CurseForge 未注册时后端返回的错误标记(见 commands.rs provider_or_err)。 */
function isCfUnconfigured(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("未配置 API Key");
}

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
  /** 点击某行的「添加/下载」按钮:传入命中项(hit.id 即 project_id)与当前选中的内容平台。缺省则不渲染该按钮,点击行进详情。 */
  onAdd?: (hit: ModpackHit, provider: ContentProvider) => void;
  /** 紧凑模式:结果区限高内滚,避免在标签页里把下方区块(已安装等)顶没。 */
  compact?: boolean;
  /** 正在安装的 project_id 集合(= hit.id);只把这些行置「安装中…」并禁用,其它行照常可点(后台并行)。 */
  addingIds?: Set<string>;
  /** 点击行主体(非按钮)时打开详情;传入命中项与当前选中的内容平台。缺省则整行点击等同 onAdd。 */
  onOpenDetail?: (hit: ModpackHit, provider: ContentProvider) => void;
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
  // 卸载时清掉未触发的防抖定时器,避免在已销毁组件上 setDebounced(无效更新 + 计时器悬挂)。
  onCleanup(() => clearTimeout(timer));

  const [results, setResults] = createSignal<SearchHit[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [loadingMore, setLoadingMore] = createSignal(false);
  const [reachedEnd, setReachedEnd] = createSignal(false);
  // 搜索失败(非「后端未连」)单独成态:区分「真的没结果」与「搜挂了」,后者给重试。
  const [searchError, setSearchError] = createSignal<string | null>(null);

  // 内容平台:默认 Modrinth。CurseForge 在切换/搜索时若返回「未配置 API Key」则禁用并提示。
  const [provider, setProvider] = createSignal<ContentProvider>("modrinth");
  const [cfUnavailable, setCfUnavailable] = createSignal(false);
  const [sort, setSort] = createSignal<SortKey>("relevance");

  function switchProvider(p: ContentProvider) {
    if (p === provider()) return;
    if (p === "curseforge" && cfUnavailable()) return;
    setProvider(p);
  }

  async function fetchPage(q: string, offset: number): Promise<SearchHit[] | null> {
    const p = provider();
    try {
      const hits = await api.modrinthSearch(
        q,
        props.kind,
        props.mcVersion || null,
        props.loader,
        PAGE,
        offset,
        p,
        sort(),
      );
      setReachedEnd(hits.length < PAGE);
      return hits;
    } catch (e) {
      if (isDesktopBackendUnavailable(e)) {
        setBackendUnavailable(true);
      } else if (p === "curseforge" && isCfUnconfigured(e)) {
        // CurseForge 未配置 key:禁用该平台、退回 Modrinth(effect 会重搜),就地提示而非反复 toast。
        setCfUnavailable(true);
        setProvider("modrinth");
      } else {
        // 翻页失败仍 toast(已有列表在);首屏失败走 searchError 占位 + 重试。
        if (offset > 0) toast({ type: "error", message: t("discover.searchFailed", { error: String(e) }) });
        else setSearchError(e instanceof Error ? e.message : String(e));
      }
      return null;
    }
  }

  // 关键词 / 类型 / 实例版本 / 加载器 / 平台 / 排序变化 → 重新拉第一页(替换)。
  createEffect(() => {
    const q = debounced();
    // 订阅以下信号,使切换实例/类型/平台/排序时也会重搜。
    void props.kind;
    void props.mcVersion;
    void props.loader;
    void provider();
    void sort();
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

  // 取值时求值 t(),避免 module-const 冻结语言。
  const SOURCES = (): { key: ContentProvider; label: string }[] => {
    const all: { key: ContentProvider; label: string }[] = [
      { key: "modrinth", label: t("discover.sourceModrinth") },
      { key: "curseforge", label: t("discover.sourceCurseforge") },
    ];
    // 整合包暂不支持从 CurseForge 安装(整合包安装路径仍是 Modrinth 专用),故该类目只暴露 Modrinth。
    return props.kind === "modpack" ? all.filter((s) => s.key === "modrinth") : all;
  };
  const SORTS = (): { key: SortKey; label: string }[] => [
    { key: "relevance", label: t("discover.sortRelevance") },
    { key: "downloads", label: t("discover.sortDownloads") },
    { key: "updated", label: t("discover.sortUpdated") },
    { key: "newest", label: t("discover.sortNewest") },
  ];

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
        placeholder={props.placeholder ?? t("discover.searchPlaceholder")}
      />

      {/* 内容平台切换(分段控件)+ 排序下拉。切换平台/排序均从 offset 0 重搜。 */}
      <div class="flex items-center justify-between gap-[12px] flex-wrap">
        <div class="inline-flex items-center gap-[2px] p-[2px] rounded-ctl bg-glass-card">
          <For each={SOURCES()}>
            {(s) => {
              const active = () => provider() === s.key;
              const cfDisabled = () => s.key === "curseforge" && cfUnavailable();
              return (
                <button
                  class="h-[26px] px-[12px] rounded-[6px] border-none text-[12px] font-medium cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app disabled:cursor-default disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
                  classList={{
                    "bg-a-4 text-white": active(),
                    "bg-transparent text-dim hover:text-fg": !active() && !cfDisabled(),
                  }}
                  disabled={cfDisabled()}
                  title={cfDisabled() ? t("discover.cfUnconfiguredHint") : ""}
                  onClick={() => switchProvider(s.key)}
                >
                  {s.label}
                </button>
              );
            }}
          </For>
        </div>

        <label class="inline-flex items-center gap-[6px] text-dim text-[12px]">
          {t("discover.sortLabel")}
          <select
            class="h-[28px] px-[8px] rounded-ctl glass-input text-fg text-[12px] cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
            value={sort()}
            onChange={(e) => setSort(e.currentTarget.value as SortKey)}
          >
            <For each={SORTS()}>
              {(o) => <option value={o.key}>{o.label}</option>}
            </For>
          </select>
        </label>
      </div>

      <Show when={cfUnavailable()}>
        <div class="text-[12px] text-dim bg-glass-card rounded-ctl px-[12px] py-[8px]">
          {t("discover.cfUnconfiguredHint")}
        </div>
      </Show>

      <Show when={!loading()} fallback={<div class="flex justify-center p-[28px]"><Spinner /></div>}>
        <Show
          when={results().length > 0}
          fallback={
            <Show
              when={!searchError()}
              fallback={
                <div class="flex flex-col items-center justify-center gap-[12px] py-[36px] text-center">
                  <div class="text-dim text-[13px]">{t("discover.searchFailedRetry")}</div>
                  <button
                    class="h-[34px] px-[16px] rounded-ctl border border-glass-border bg-glass-card text-fg text-[13px] cursor-pointer transition-[background-color] duration-[var(--dur)] ease-app hover:bg-glass-hover"
                    onClick={retry}
                  >
                    {t("discover.retry")}
                  </button>
                </div>
              }
            >
            <div class="p-[24px] text-dim text-center text-[13px]">
              <Show
                when={!backendUnavailable()}
                fallback={t("discover.backendUnavailable")}
              >
                {debounced().trim() ? t("discover.noResults") : t("discover.enterKeyword")}
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
                const onOpenDetail = props.onOpenDetail;
                const open = onOpenDetail
                  ? (h: ModpackHit) => onOpenDetail(h, provider())
                  : onAdd
                    ? (h: ModpackHit) => onAdd(h, provider())
                    : () => {};
                return (
                  <ModpackListItem
                    hit={hit}
                    onClick={open}
                    action={
                      onAdd ? (
                        <button
                          class={added() ? ADDED_BTN : ADD_BTN}
                          disabled={disabled()}
                          title={reason() ?? ""}
                          onClick={() => onAdd(hit, provider())}
                        >
                          {added() ? t("discover.added") : busy() ? t("discover.installing") : t("discover.add")}
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
                {loadingMore() ? t("discover.loadingMore") : t("discover.loadMore")}
              </button>
            </div>
          </Show>
        </Show>
      </Show>
    </div>
  );
};

export default ContentBrowser;
