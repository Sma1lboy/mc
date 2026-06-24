import { Component, createSignal, createEffect, createResource, onMount, on, Show } from "solid-js";
import { ContentBrowser, Spinner, type ModpackHit } from "../components";
import { FacetSidebar, type FacetSelection } from "../components/FacetSidebar";
import type { ContentProvider } from "../components/ContentBrowser";
import type { ProjectKind } from "../ipc/types";
import { api } from "../ipc/api";
import { prefetchKinds } from "../util/contentSearch";
import { discoverKind, setDiscoverKind, DISCOVER_KINDS, discoverTarget, setDiscoverTarget } from "../store";
import { t } from "../i18n";
import ModpackDetail from "./ModpackDetail";
import ProjectInstallDetail from "./ProjectInstallDetail";

/**
 * Discover —— Modrinth 搜索页。类型切换 + facet 侧栏 + 防抖搜索 + 列表。
 * 浏览态为两栏:左 <FacetSidebar>(多选 facet,固定宽可滚),右 <ContentBrowser>(搜索/分页/列表)。
 * facet 选择上提到本页,既喂给侧栏渲染/编辑,也喂给浏览器搜索(改 facet 即从 offset 0 重搜)。
 * Discover 不绑定实例,故不按版本/加载器过滤(mcVersion="" + loader=null),点击行打开详情页而非直接下载。
 * 详情态(ProjectInstallDetail / ModpackDetail)整页全宽,不带侧栏。
 */

type SelectedProject = { hit: ModpackHit; kind: ProjectKind; provider: ContentProvider };

const EMPTY_FACETS: FacetSelection = { categories: [], loaders: [], gameVersions: [], environment: null, openSource: false };

const Discover: Component = () => {
  // 类型状态在 store(顶栏 TopBar 的类型标签与本页共享同一信号)。
  const kind = discoverKind;

  // 当前打开详情的项目(null = 显示搜索网格)。点击卡片/按钮进入详情页,而非直接下载。
  const [selected, setSelected] = createSignal<SelectedProject | null>(null);

  // 浏览态当前内容平台:由 ContentBrowser 内部切换时上报,用来决定侧栏显示哪些 facet。
  const [provider, setProvider] = createSignal<ContentProvider>("modrinth");

  // facet 多选选择(上提:侧栏渲染/编辑 + 浏览器搜索)。切类型时重置(分类/加载器随类型变)。
  const [facets, setFacets] = createSignal<FacetSelection>(EMPTY_FACETS);

  // 分类法统一在此拉取(进程内缓存),下传侧栏;与首屏搜索一起决定「整体就绪」。
  const [facetTags] = createResource(() => api.contentFacets());
  // 首屏搜索是否已就绪(ContentBrowser 首次 loading→false 时置真);切类型时重置。
  const [seeded, setSeeded] = createSignal(false);
  const viewReady = () => !facetTags.loading && seeded();

  // 进入发现页即后台预取所有类型的默认首屏(空搜索 / Modrinth),切换类型时直接命中缓存、即时显示。
  onMount(() => prefetchKinds(DISCOVER_KINDS));

  function openHit(h: ModpackHit, prov: ContentProvider) {
    setSelected({ hit: h, kind: kind(), provider: prov });
  }

  // 顶栏切换类型 → 重置筛选 / 首屏就绪态,并关掉可能打开的详情(回到该类型的浏览)。
  // openingDetail:从首页卡片跳转时既改类型又开详情,这里别把刚开的详情清掉。
  let openingDetail = false;
  createEffect(
    on(
      discoverKind,
      () => {
        setFacets(EMPTY_FACETS);
        setSeeded(false);
        if (!openingDetail) setSelected(null);
        openingDetail = false;
      },
      { defer: true },
    ),
  );

  // 从首页「发现」卡片跳进来时,切到目标类型并直接打开其详情(消费一次即清空)。
  createEffect(() => {
    const tgt = discoverTarget();
    if (!tgt) return;
    setDiscoverTarget(null);
    if (tgt.kind !== discoverKind()) openingDetail = true;
    setDiscoverKind(tgt.kind);
    setSelected({ hit: tgt.hit, kind: tgt.kind, provider: "modrinth" });
  });

  return (
    <div class="px-[28px] py-[24px] overflow-y-auto h-full">
      <Show when={selected()}>
        {(project) => (
          <Show
            when={project().kind === "modpack"}
            fallback={
              <ProjectInstallDetail
                hit={project().hit}
                kind={project().kind as Exclude<ProjectKind, "modpack">}
                provider={project().provider}
                onBack={() => setSelected(null)}
              />
            }
          >
            <ModpackDetail hit={project().hit} provider={project().provider} onBack={() => setSelected(null)} />
          </Show>
        )}
      </Show>

      <Show when={!selected()}>
        {/* 类型标签已上提到顶栏 TopBar;此处只剩筛选 + 内容。切类型时整栏重挂(清空搜索词/分页,
            facet 在 kind 变化的副作用里重置)。Discover 不绑定实例:mcVersion="" + loader=null。 */}
        <Show when={kind()} keyed>
          {(k) => (
            <div class="relative">
              {/* 整体就绪(facet 分类法 + 首屏搜索都好)前盖一个统一 spinner,避免两栏错峰出现。 */}
              <Show when={!viewReady()}>
                <div class="absolute inset-0 z-10 flex items-start justify-center pt-[60px]">
                  <Spinner />
                </div>
              </Show>
              <div class="flex gap-[20px] items-start" classList={{ invisible: !viewReady() }}>
                <FacetSidebar
                  kind={k}
                  provider={provider()}
                  selected={facets}
                  onChange={setFacets}
                  tags={facetTags}
                />
                <div class="flex-1 min-w-0">
                  <ContentBrowser
                    kind={k}
                    mcVersion=""
                    loader={null}
                    onAdd={openHit}
                    onOpenDetail={openHit}
                    placeholder={t("discover.searchPlaceholder")}
                    onProviderChange={setProvider}
                    onLoadingChange={(l) => {
                      if (!l) setSeeded(true);
                    }}
                    categories={() => facets().categories}
                    loaders={() => facets().loaders}
                    gameVersions={() => facets().gameVersions}
                    environment={() => facets().environment}
                    openSource={() => facets().openSource}
                  />
                </div>
              </div>
            </div>
          )}
        </Show>
      </Show>
    </div>
  );
};

export default Discover;
