import { useEffect, useMemo, useRef, useState } from "react";
import clsx from "clsx";
import { ContentBrowser, BlockedFilesDialog, Spinner, Heading, Chip, toast, type ModpackHit } from "../components";
import { FacetSidebar, type FacetSelection } from "../components/FacetSidebar";
import type { ContentProvider } from "../components/ContentBrowser";
import type { ProjectKind, ImportOutcome } from "../ipc/types";
import { api } from "../ipc/api";
import { cached } from "../ipc/cache";
import { prefetchKinds } from "../util/contentSearch";
import { useAsync } from "../util/useAsync";
import { enqueueDownload, downloadForRef, fractionOf, useTasks, type DownloadTask } from "../util/downloads";
import {
  useAppStore,
  activeRoot,
  setDiscoverKind,
  DISCOVER_KINDS,
  setDiscoverTarget,
  refreshInstances,
} from "../store";
import { t, useLang } from "../i18n";
import { openAgentChat, discoverPrompt } from "../agent/chatStore";
import ModpackDetail from "./ModpackDetail";
import ProjectInstallDetail from "./ProjectInstallDetail";

/**
 * Discover —— 内容发现页。顶部工具栏(标题 + 内容类型 Chips 行)+ <ContentBrowser>。
 * Blocky Craft 改造:取消整列 facet 侧栏,facet 收进 ContentBrowser 工具条的可移除芯片
 * + 「更多筛选」弹层;源切换(Modrinth/CurseForge)与排序也在 ContentBrowser 内。
 * facet 选择上提到本页(喂弹层渲染/编辑 + 浏览器搜索;改 facet 即从 offset 0 重搜)。
 * Discover 不绑定实例,故不按版本/加载器过滤(mcVersion="" + loader=null),点击行打开详情页而非直接下载。
 * 详情态(ProjectInstallDetail / ModpackDetail)整页全宽。
 */

type SelectedProject = { hit: ModpackHit; kind: ProjectKind; provider: ContentProvider };

/** 内容类型 → i18n 标签键(类型 Chips 行)。 */
function kindLabelKey(k: ProjectKind): string {
  switch (k) {
    case "modpack":
      return "discover.kindModpack";
    case "mod":
      return "discover.kindMod";
    case "shader":
      return "discover.kindShader";
    case "resourcepack":
      return "discover.kindResourcepack";
    case "datapack":
      return "discover.kindDatapack";
    default:
      return "discover.kindModpack";
  }
}

const EMPTY_FACETS: FacetSelection = { categories: [], loaders: [], gameVersions: [], environment: null, openSource: false };

export default function Discover() {
  useLang();

  // 类型状态在 store(顶栏 TopBar 的类型标签与本页共享同一信号)。
  const kind = useAppStore((s) => s.discoverKind);
  const discoverTarget = useAppStore((s) => s.discoverTarget);

  // 当前打开详情的项目(null = 显示搜索网格)。点击卡片/按钮进入详情页,而非直接下载。
  const [selected, setSelected] = useState<SelectedProject | null>(null);

  // facet 多选选择(上提:侧栏渲染/编辑 + 浏览器搜索)。切类型时重置(分类/加载器随类型变)。
  const [facets, setFacets] = useState<FacetSelection>(EMPTY_FACETS);
  // 当前内容平台(由 ContentBrowser 内部切换上报);决定左侧筛选栏显示哪些 facet 组。
  const [provider, setProvider] = useState<ContentProvider>("modrinth");

  // 分类法统一在此拉取(进程内缓存),下传侧栏;与首屏搜索一起决定「整体就绪」。
  const facetTags = useAsync(() => api.contentFacets(), []);
  // 首屏搜索是否已就绪(ContentBrowser 首次 loading→false 时置真);切类型时重置。
  const [seeded, setSeeded] = useState(false);
  const viewReady = !facetTags.loading && seeded;

  // 进入发现页即后台预取所有类型的默认首屏(空搜索 / Modrinth),切换类型时直接命中缓存、即时显示。
  useEffect(() => {
    prefetchKinds(DISCOVER_KINDS);
  }, []);

  function openHit(h: ModpackHit, prov: ContentProvider) {
    setSelected({ hit: h, kind, provider: prov });
  }

  // 列表「添加」:整合包经全局下载队列直接装最新版(顶栏队列可见 + 行内进度条);
  // 其它类型没有目标实例,进详情选实例装。安装中 / 已添加 / 进度都由队列派生(以 hit.id 为 refId),
  // 与顶栏面板同一份状态。
  const [outcome, setOutcome] = useState<ImportOutcome | null>(null);

  const tasks: DownloadTask[] = useTasks();
  const installing = useMemo(
    () =>
      new Set(
        tasks
          .filter((dl) => (dl.status === "active" || dl.status === "queued") && dl.refId)
          .map((dl) => dl.refId!),
      ),
    [tasks],
  );
  const added = useMemo(
    () =>
      new Set(
        tasks
          .filter((dl) => dl.status === "done" && dl.refId)
          .map((dl) => dl.refId!),
      ),
    [tasks],
  );
  const progressOf = (id: string): number | null | undefined => {
    const task = downloadForRef(id);
    if (!task || task.status === "done" || task.status === "error") return undefined;
    return fractionOf(task);
  };

  function quickAdd(hit: ModpackHit, prov: ContentProvider) {
    if (kind !== "modpack") {
      openHit(hit, prov); // 非整合包:没有目标实例,进详情选实例安装
      return;
    }
    const existing = downloadForRef(hit.id);
    if (existing && existing.status !== "error") return; // 已在队列 / 已装,避免重复
    enqueueDownload({
      refId: hit.id,
      title: hit.title,
      icon: hit.icon_url ?? null,
      kind: "modpack",
      run: async () => {
        const versions = await cached(`versions|${prov}|${hit.id}`, () => api.modrinthVersions(hit.id, prov));
        const latest = versions[0];
        if (!latest) throw new Error(t("discover.noVersions"));
        return api.installModpack(activeRoot(), prov, hit.id, latest.id, null, hit.icon_url ?? null);
      },
      onComplete: (result) => {
        const out = result as ImportOutcome;
        refreshInstances(); // 新建了实例,库 / 侧栏 / 首页同步
        if (out.blocked.length > 0 || out.skipped_optional.length > 0) setOutcome(out);
        else toast({ type: "success", message: t("discover.installedModpack", { id: out.instance_id }) });
      },
      onError: (e) => toast({ type: "error", message: t("discover.installFailed", { error: String(e) }) }),
    });
    toast({ type: "info", message: t("discover.installQueued", { title: hit.title }) });
  }

  // 顶栏切换类型 → 重置筛选 / 首屏就绪态,并关掉可能打开的详情(回到该类型的浏览)。
  // openingDetail:从首页卡片跳转时既改类型又开详情,这里别把刚开的详情清掉。
  const openingDetail = useRef(false);
  const kindMounted = useRef(false);
  useEffect(() => {
    // defer:跳过挂载首跑,仅在 kind 真正切换时执行(对齐 Solid on(..., { defer: true }))。
    if (!kindMounted.current) {
      kindMounted.current = true;
      return;
    }
    setFacets(EMPTY_FACETS);
    setSeeded(false);
    if (!openingDetail.current) setSelected(null);
    openingDetail.current = false;
  }, [kind]);

  // 从首页「发现」卡片跳进来时,切到目标类型并直接打开其详情(消费一次即清空)。
  useEffect(() => {
    if (!discoverTarget) return;
    setDiscoverTarget(null);
    if (discoverTarget.kind !== kind) openingDetail.current = true;
    setDiscoverKind(discoverTarget.kind);
    setSelected({ hit: discoverTarget.hit, kind: discoverTarget.kind, provider: "modrinth" });
  }, [discoverTarget]); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="h-full overflow-hidden">
      {/* 详情态(ProjectInstallDetail / ModpackDetail)按需挂载/卸载,自带滚动容器,盖在浏览器之上。
          项目/版本走 cached() 故重挂无代价。 */}
      {selected && (
        <div className="h-full overflow-y-auto px-[28px] py-[24px]">
          {selected.kind === "modpack" ? (
            <ModpackDetail hit={selected.hit} provider={selected.provider} onBack={() => setSelected(null)} />
          ) : (
            <ProjectInstallDetail
              hit={selected.hit}
              kind={selected.kind as Exclude<ProjectKind, "modpack">}
              provider={selected.provider}
              onBack={() => setSelected(null)}
            />
          )}
        </div>
      )}

      {/* 浏览器子树常驻挂载:打开详情时仅以 display:none 隐藏(不卸载),返回即原样恢复
          搜索词 / 结果 / 分页 / 排序 / 源 / 滚动位置(独立滚动容器,scrollTop 被 display:none 保留)。
          顶部工具栏:页面标题 + 内容类型 Chips 行(选中橙;接 discoverKind/DISCOVER_KINDS)。
          源切换(Modrinth/CurseForge)、搜索、排序、可移除筛选芯片与「更多筛选」弹层都在
          <ContentBrowser> 内。切类型时内容区整栏重挂(清空搜索词/分页,facet 在 kind 副作用里重置)。
          Discover 不绑定实例:mcVersion="" + loader=null。 */}
      <div className={clsx("h-full overflow-y-auto px-[28px] py-[24px]", { hidden: !!selected })}>
        <div className="flex flex-col gap-[16px]">
          <Heading size="section" as="h1">
            {t("discover.pageTitle")}
          </Heading>

          <div className="flex items-center gap-[8px] flex-wrap">
            {DISCOVER_KINDS.map((k) => (
              <Chip key={k} active={kind === k} onClick={() => setDiscoverKind(k)}>
                {t(kindLabelKey(k))}
              </Chip>
            ))}
          </div>

          {/* 切类型时内容区整栏重挂(key={kind},对齐 Solid <Show keyed>)。 */}
          <div key={kind} className="relative">
            {/* 整体就绪(facet 分类法 + 首屏搜索都好)前盖一个统一 spinner,避免错峰出现。 */}
            {!viewReady && (
              <div className="absolute inset-0 z-10 flex items-start justify-center pt-[60px]">
                <Spinner />
              </div>
            )}
            <div className={clsx("flex gap-[20px] items-start", { invisible: !viewReady })}>
              <FacetSidebar
                kind={kind}
                provider={provider}
                selected={facets}
                onChange={setFacets}
                tags={facetTags.data}
              />
              <div className="flex-1 min-w-0">
                <ContentBrowser
                  kind={kind}
                  mcVersion=""
                  loader={null}
                  onAdd={quickAdd}
                  onOpenDetail={openHit}
                  addingIds={installing}
                  addedIds={added}
                  progressOf={progressOf}
                  placeholder={t("discover.searchPlaceholder")}
                  onLoadingChange={(l) => {
                    if (!l) setSeeded(true);
                  }}
                  onProviderChange={setProvider}
                  onAskAgent={(query) =>
                    openAgentChat(
                      discoverPrompt(query, facets.gameVersions[0] ?? null, facets.loaders[0] ?? null),
                    )
                  }
                  categories={facets.categories}
                  loaders={facets.loaders}
                  gameVersions={facets.gameVersions}
                  environment={facets.environment}
                  openSource={facets.openSource}
                />
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* 整合包从列表直接安装时,若有需手动下载 / 被跳过的文件,弹窗摊开。 */}
      {outcome && (
        <BlockedFilesDialog
          instanceId={outcome.instance_id}
          blocked={outcome.blocked}
          skipped={outcome.skipped_optional}
          onClose={() => setOutcome(null)}
        />
      )}
    </div>
  );
}
