import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import {
  BlockedFilesDialog,
  EmptyState,
  ErrorState,
  ExportModpackDialog,
  Icon,
  ImportModpackDialog,
  InstanceRow,
  ModpackCard,
  Spinner,
  toast,
  type InstanceRowData,
  type ModpackHit,
} from "../components";
import { api, onLaunchProgress } from "../ipc/api";
import { activeRoot, openInstance, setCurrentPage, openDiscover, playInstance, instances, refreshInstances } from "../store";
import { openInstanceDir, deleteInstance } from "../util/instanceActions";
import { useModpackDrop } from "../util/useModpackDrop";
import { sortByRecent } from "../util/instances";
import { t } from "../i18n";
import type { ImportOutcome, InstanceSummary, SearchHit } from "../ipc/types";

/**
 * Home —— 工作台 dashboard。
 *   - "Jump back in":当前根目录下按最近游玩排序的实例,带 Play。
 *   - "Discover a modpack":Modrinth 整合包大卡网格。
 * 数据全部经 createResource 拉取,自动随 currentRoot 变化重新加载。
 */

function toRowData(i: InstanceSummary): InstanceRowData {
  return {
    id: i.id,
    name: i.name || i.id,
    mc_version: i.mc_version,
    loader: i.loader,
    loader_version: i.loader_version || undefined,
    icon: i.icon || undefined,
    last_played: i.last_played ?? 0,
    running: i.running ?? false,
  };
}

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

const Home: Component = () => {
  // 实例列表来自全局 store(单一真相,见 store.ts);依赖 currentRoot,切根自动重拉。

  // 整合包推荐:一次性拉取热门 modpack。
  const [packs] = createResource(() =>
    api.modrinthSearch("", "modpack", null, null, null, null, null, null, null).catch(() => [] as SearchHit[]),
  );

  // 导出整合包:选格式弹窗(非空 = 打开,目标实例即其值)。
  const [exportRow, setExportRow] = createSignal<InstanceRowData | null>(null);

  // 导入整合包:把文件拖到本页任意处即导入(打开弹窗并自动开始);弹窗已开时让它自己接管。
  const [importOpen, setImportOpen] = createSignal(false);
  const [importPath, setImportPath] = createSignal<string | null>(null);
  const [importOutcome, setImportOutcome] = createSignal<ImportOutcome | null>(null);
  function handleImported(out: ImportOutcome) {
    refreshInstances();
    if (out.blocked.length > 0 || out.skipped_optional.length > 0) setImportOutcome(out);
    else toast({ type: "success", message: t("library.imported", { id: out.instance_id }) });
  }
  const dragOver = useModpackDrop({
    enabled: () => !importOpen(),
    onFile: (path) => {
      setImportPath(path);
      setImportOpen(true);
    },
    onUnsupported: () => toast({ type: "info", message: t("components.import.unsupported") }),
  });

  // 启动反馈:仅订阅进度提示。成功/退出/崩溃的 toast 与运行态由 store 统一处理
  //(基于真实的 game://started/exit 事件,而非「第一行日志」这种会把崩溃误报成成功的信号)。
  // 后端可能把同一启动 stage 连发多次;去重,避免「启动游戏进程」弹两遍。
  let lastStage = "";
  const offProgress = onLaunchProgress((p) => {
    if (p.stage && p.stage !== lastStage) {
      lastStage = p.stage;
      toast({ type: "info", message: p.stage });
    }
  });
  onCleanup(() => {
    offProgress();
  });

  // Home 只当快捷入口:最近游玩取前 5;完整列表在「库」页(下方「查看全部」跳转)。
  const RECENT_CAP = 5;
  const sortedByPlayed = () => sortByRecent(instances() ?? []);
  const recent = () => sortedByPlayed().slice(0, RECENT_CAP);

  return (
    <div class="relative py-[24px] px-[28px] overflow-y-auto h-full">
      <Show when={dragOver()}>
        <div class="absolute inset-0 z-30 flex items-center justify-center bg-black/40 backdrop-blur-sm pointer-events-none">
          <div class="flex flex-col items-center gap-[10px] rounded-card border-2 border-dashed border-a-4 bg-glass-card px-[40px] py-[32px]">
            <Icon name="download" size={30} class="text-a-5" />
            <div class="text-[14px] font-semibold text-fg">{t("components.import.dropOverlay")}</div>
          </div>
        </div>
      </Show>
      <header class="mb-[20px]">
        <h1 class="text-[28px] font-bold text-fg m-0">{t("library.welcomeBack")}</h1>
      </header>

      <section>
        <div class="flex items-center justify-between mb-[14px]">
          <h2 class="text-[18px] font-semibold text-fg m-0">{t("library.continuePlaying")}</h2>
          <Show when={sortedByPlayed().length > RECENT_CAP}>
            <button
              class="text-[13px] text-dim bg-transparent border-none cursor-pointer transition-colors duration-150 hover:text-fg"
              onClick={() => setCurrentPage("library")}
            >
              {t("library.viewAll")}
            </button>
          </Show>
        </div>
        <Show
          when={!instances.loading}
          fallback={<div class="flex justify-center p-[32px]"><Spinner /></div>}
        >
          <Show
            when={!instances.error}
            fallback={
              <ErrorState message={t("library.instanceListError")} onRetry={() => void refreshInstances()} />
            }
          >
          <Show
            when={recent().length > 0}
            fallback={
              <EmptyState title={<>{t("library.emptyHomePrefix")}<b>{t("library.emptyHomeLink")}</b>{t("library.emptyHomeSuffix")}</>} />
            }
          >
            <div class="flex flex-col gap-[10px]">
              <For each={recent()}>
                {(inst) => (
                  <InstanceRow
                    instance={toRowData(inst)}
                    onPlay={playInstance}
                    onOpen={openInstance}
                    onManage={openInstance}
                    onOpenDir={(id) => void openInstanceDir(activeRoot(), id)}
                    onExport={() => setExportRow(toRowData(inst))}
                    onDelete={async (id) => {
                      if (await deleteInstance(activeRoot(), { id, name: toRowData(inst).name }))
                        refreshInstances();
                    }}
                  />
                )}
              </For>
            </div>
          </Show>
          </Show>
        </Show>
      </section>

      <section class="mt-[28px]">
        <button
          class="bg-transparent border-none p-0 mb-[14px] text-[18px] font-semibold text-fg cursor-pointer hover:text-a-5 transition-colors duration-150"
          onClick={() => openDiscover()}
        >
          {t("library.discoverModpack")}
        </button>
        <Show
          when={!packs.loading}
          fallback={<div class="flex justify-center p-[32px]"><Spinner /></div>}
        >
          <div class="grid grid-cols-2 gap-[16px]">
            <For each={(packs() ?? []).slice(0, 6)}>
              {(hit) => (
                <ModpackCard
                  hit={toHit(hit)}
                  onClick={(h) => openDiscover({ hit: h, kind: "modpack" })}
                />
              )}
            </For>
          </div>
        </Show>
      </section>

      <ExportModpackDialog
        open={!!exportRow()}
        root={activeRoot()}
        instance={exportRow()}
        onClose={() => setExportRow(null)}
      />

      <ImportModpackDialog
        open={importOpen()}
        root={activeRoot()}
        initialPath={importPath()}
        onClose={() => {
          setImportOpen(false);
          setImportPath(null);
        }}
        onImported={handleImported}
      />

      <Show when={importOutcome()}>
        {(o) => (
          <BlockedFilesDialog
            instanceId={o().instance_id}
            blocked={o().blocked}
            skipped={o().skipped_optional}
            onClose={() => setImportOutcome(null)}
          />
        )}
      </Show>
    </div>
  );
};

export default Home;
