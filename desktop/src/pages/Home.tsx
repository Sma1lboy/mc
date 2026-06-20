import { Component, createResource, For, Show, onCleanup } from "solid-js";
import {
  InstanceRow,
  ModpackCard,
  Spinner,
  toast,
  type InstanceRowData,
  type ModpackHit,
} from "../components";
import { api, onGameLog, onLaunchProgress } from "../ipc/api";
import { currentRoot } from "../store";
import { openInstanceDir, exportInstanceMrpack, deleteInstance } from "../util/instanceActions";
import type { InstanceSummary, SearchHit } from "../ipc/types";

/**
 * Home —— Modrinth 式 dashboard。
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
    running: i.running,
  };
}

function toHit(h: SearchHit): ModpackHit {
  return {
    id: (h as any).id ?? h.project_id,
    slug: h.slug,
    title: h.title,
    description: h.description,
    author: h.author,
    downloads: h.downloads,
    icon_url: h.icon_url || undefined,
    gallery_url: (h as any).gallery_url || undefined,
    categories: h.categories,
  };
}

const Home: Component = () => {
  // 实例列表:依赖 currentRoot,空根目录用 "" 让后端落到默认根。
  const [instances, { refetch: refetchInstances }] = createResource(
    () => currentRoot() ?? "",
    (root) => api.listInstances(root),
  );

  // 整合包推荐:一次性拉取热门 modpack。
  const [packs] = createResource(() =>
    api.modrinthSearch("", "modpack", null, null).catch(() => [] as SearchHit[]),
  );

  // 启动反馈:订阅进度与日志,给出 toast。
  const offProgress = onLaunchProgress((p) => {
    if (p.stage) toast({ type: "info", message: p.stage });
  });
  let firstLog = true;
  const offLog = onGameLog(() => {
    if (firstLog) {
      firstLog = false;
      toast({ type: "success", message: "游戏已启动" });
    }
  });
  onCleanup(() => {
    offProgress();
    offLog();
  });

  const recent = () =>
    [...(instances() ?? [])]
      .sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0))
      .slice(0, 6);

  async function play(id: string) {
    try {
      firstLog = true;
      await api.launchInstance(currentRoot() ?? "", id, "Player", false);
    } catch (e) {
      toast({ type: "error", message: `启动失败:${e}` });
    }
  }

  return (
    <div class="py-[24px] px-[28px] overflow-y-auto h-full">
      <header>
        <h1 class="text-[28px] font-bold text-fg m-0 mb-[4px]">Welcome back!</h1>
        <h2 class="text-[18px] font-semibold text-fg my-[12px]">Jump back in</h2>
      </header>

      <Show
        when={!instances.loading}
        fallback={<div class="flex justify-center p-[32px]"><Spinner /></div>}
      >
        <Show
          when={recent().length > 0}
          fallback={
            <div class="p-[24px] rounded-card bg-n-3 text-dim text-center">
              还没有实例。去 <b>库 / Discover</b> 安装一个版本开始游玩。
            </div>
          }
        >
          <div class="flex flex-col gap-[10px]">
            <For each={recent()}>
              {(inst) => (
                <InstanceRow
                  instance={toRowData(inst)}
                  onPlay={play}
                  onOpenDir={(id) => void openInstanceDir(currentRoot() ?? "", id)}
                  onExport={() => void exportInstanceMrpack(currentRoot() ?? "", toRowData(inst))}
                  onDelete={async (id) => {
                    if (await deleteInstance(currentRoot() ?? "", { id, name: toRowData(inst).name }))
                      refetchInstances();
                  }}
                />
              )}
            </For>
          </div>
        </Show>
      </Show>

      <section class="mt-[28px]">
        <h2 class="text-[18px] font-semibold text-fg m-0 mb-[14px] cursor-pointer">Discover a modpack →</h2>
        <Show
          when={!packs.loading}
          fallback={<div class="flex justify-center p-[32px]"><Spinner /></div>}
        >
          <div class="grid grid-cols-2 gap-[16px]">
            <For each={(packs() ?? []).slice(0, 6)}>
              {(hit) => (
                <ModpackCard
                  hit={toHit(hit)}
                  onClick={(h) => toast({ type: "info", message: `打开 ${h.title}` })}
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
