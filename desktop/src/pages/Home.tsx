import { Component, createResource, For, Show, onCleanup } from "solid-js";
import {
  EmptyState,
  InstanceRow,
  ModpackCard,
  Spinner,
  toast,
  type InstanceRowData,
  type ModpackHit,
} from "../components";
import { api, onLaunchProgress } from "../ipc/api";
import { activeRoot, openInstance, setCurrentPage, openDiscover, playInstance } from "../store";
import { openInstanceDir, exportInstanceMrpack, deleteInstance } from "../util/instanceActions";
import type { InstanceSummary, SearchHit } from "../ipc/types";

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
  // 实例列表:依赖 currentRoot,空根目录用 "" 让后端落到默认根。
  const [instances, { refetch: refetchInstances }] = createResource(
    () => activeRoot(),
    (root) => api.listInstances(root),
  );

  // 整合包推荐:一次性拉取热门 modpack。
  const [packs] = createResource(() =>
    api.modrinthSearch("", "modpack", null, null, null, null).catch(() => [] as SearchHit[]),
  );

  // 启动反馈:仅订阅进度提示。成功/退出/崩溃的 toast 与运行态由 store 统一处理
  //(基于真实的 game://started/exit 事件,而非「第一行日志」这种会把崩溃误报成成功的信号)。
  const offProgress = onLaunchProgress((p) => {
    if (p.stage) toast({ type: "info", message: p.stage });
  });
  onCleanup(() => {
    offProgress();
  });

  // Home 只当快捷入口:最近游玩取前 5;完整列表在「库」页(下方「查看全部」跳转)。
  const RECENT_CAP = 5;
  const sortedByPlayed = () =>
    [...(instances() ?? [])].sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0));
  const recent = () => sortedByPlayed().slice(0, RECENT_CAP);

  return (
    <div class="py-[24px] px-[28px] overflow-y-auto h-full">
      <header class="mb-[20px]">
        <h1 class="text-[28px] font-bold text-fg m-0">Welcome back!</h1>
      </header>

      <section>
        <div class="flex items-center justify-between mb-[14px]">
          <h2 class="text-[18px] font-semibold text-fg m-0">Jump back in</h2>
          <Show when={sortedByPlayed().length > RECENT_CAP}>
            <button
              class="text-[13px] text-dim bg-transparent border-none cursor-pointer transition-colors duration-150 hover:text-fg"
              onClick={() => setCurrentPage("library")}
            >
              查看全部 →
            </button>
          </Show>
        </div>
        <Show
          when={!instances.loading}
          fallback={<div class="flex justify-center p-[32px]"><Spinner /></div>}
        >
          <Show
            when={recent().length > 0}
            fallback={
              <EmptyState title={<>还没有实例。去 <b>库 / Discover</b> 安装一个版本开始游玩。</>} />
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
                    onExport={() => void exportInstanceMrpack(activeRoot(), toRowData(inst))}
                    onDelete={async (id) => {
                      if (await deleteInstance(activeRoot(), { id, name: toRowData(inst).name }))
                        refetchInstances();
                    }}
                  />
                )}
              </For>
            </div>
          </Show>
        </Show>
      </section>

      <section class="mt-[28px]">
        <button
          class="bg-transparent border-none p-0 mb-[14px] text-[18px] font-semibold text-fg cursor-pointer hover:text-a-5 transition-colors duration-150"
          onClick={() => openDiscover()}
        >
          Discover a modpack →
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

    </div>
  );
};

export default Home;
