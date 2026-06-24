import { Component, createSignal, createEffect, createResource, onMount, For, Show } from "solid-js";
import { ContentBrowser, Spinner, type ModpackHit } from "../components";
import { FacetSidebar, type FacetSelection } from "../components/FacetSidebar";
import type { ContentProvider } from "../components/ContentBrowser";
import type { ProjectKind } from "../ipc/types";
import { api } from "../ipc/api";
import { prefetchKinds } from "../util/contentSearch";
import { discoverTarget, setDiscoverTarget } from "../store";
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

const KINDS = (): { key: ProjectKind; label: string }[] => [
  { key: "modpack", label: t("discover.kindModpack") },
  { key: "mod", label: t("discover.kindMod") },
  { key: "shader", label: t("discover.kindShader") },
  { key: "resourcepack", label: t("discover.kindResourcepack") },
  { key: "datapack", label: t("discover.kindDatapack") },
];

type SelectedProject = { hit: ModpackHit; kind: ProjectKind; provider: ContentProvider };

const EMPTY_FACETS: FacetSelection = { categories: [], loaders: [], gameVersions: [], environment: null, openSource: false };

const Discover: Component = () => {
  const [kind, setKind] = createSignal<ProjectKind>("modpack");

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
  onMount(() => prefetchKinds(KINDS().map((k) => k.key)));

  function switchKind(k: ProjectKind) {
    if (k === kind()) return;
    setSeeded(false);
    setKind(k);
    setFacets(EMPTY_FACETS);
  }

  function openHit(h: ModpackHit, prov: ContentProvider) {
    setSelected({ hit: h, kind: kind(), provider: prov });
  }

  // 从首页「发现」卡片跳进来时,自动打开目标项目详情(消费一次即清空)。
  createEffect(() => {
    const tgt = discoverTarget();
    if (!tgt) return;
    setKind(tgt.kind);
    setFacets(EMPTY_FACETS);
    setSelected({ hit: tgt.hit, kind: tgt.kind, provider: "modrinth" });
    setDiscoverTarget(null);
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
        <div class="flex items-center justify-between gap-[16px] mb-[16px]">
          <h1 class="text-[24px] font-bold text-fg m-0">{t("discover.heading")}</h1>
        </div>

        <div class="flex gap-[8px] mb-[16px]">
          <For each={KINDS()}>
            {(k) => (
              <button
                class="px-[14px] py-[6px] border-none rounded-ctl text-[13px] cursor-pointer transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5 focus-visible:ring-offset-2 focus-visible:ring-offset-n-1"
                classList={{
                  "bg-a-4 text-white": kind() === k.key,
                  "bg-glass-card text-dim hover:bg-glass-hover hover:text-fg": kind() !== k.key,
                }}
                onClick={() => switchKind(k.key)}
              >
                {k.label}
              </button>
            )}
          </For>
        </div>

        {/* 两栏:左 facet 侧栏 + 右内容浏览器。切类型时整体重挂(清空搜索词/分页 + facet 已在 switchKind 重置)。
            Discover 不绑定实例:mcVersion="" + loader=null;点击行或「添加」均打开详情页。 */}
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
