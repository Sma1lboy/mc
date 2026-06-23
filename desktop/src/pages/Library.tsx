import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import {
  InstanceRow,
  Button,
  EmptyState,
  ErrorState,
  Spinner,
  SearchBox,
  toast,
  type InstanceRowData,
} from "../components";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot, openInstance, playInstance } from "../store";
import { openInstanceDir, exportInstanceMrpack, deleteInstance } from "../util/instanceActions";
import type { InstanceSummary, ManifestVersion } from "../ipc/types";

/**
 * Library —— 当前根目录的全部实例 + "安装新版本" 抽屉。
 * 安装进度通过 install://progress 事件实时显示在按钮旁。
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

const Library: Component = () => {
  const [instances, { refetch }] = createResource(
    () => activeRoot(),
    (root) => api.listInstances(root),
  );
  const [versions, { refetch: refetchVersions }] = createResource(() => api.listVersions(false));

  const [showInstall, setShowInstall] = createSignal(false);
  const [filter, setFilter] = createSignal("");
  // 实例列表过滤(名称/版本/加载器),与版本安装抽屉的搜索相互独立。
  const [instQuery, setInstQuery] = createSignal("");
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [progress, setProgress] = createSignal("");

  const off = onInstallProgress((p) => {
    if (p.total > 0) setProgress(`${p.stage} ${p.current}/${p.total}`);
    else setProgress(p.stage);
  });
  onCleanup(off);

  const filtered = () => {
    const q = filter().toLowerCase();
    return (versions() ?? []).filter((v) => v.id.toLowerCase().includes(q)).slice(0, 60);
  };

  // 默认按上次游玩降序(最近玩的在前,与首页「继续游玩」同序);未玩过的(0)沉底。
  const sortedInstances = () =>
    [...(instances() ?? [])].sort((a, b) => (b.last_played ?? 0) - (a.last_played ?? 0));

  // 在排序基础上按名称 / 版本 / 加载器过滤(空查询返回全部)。
  const filteredInstances = () => {
    const all = sortedInstances();
    const q = instQuery().trim().toLowerCase();
    if (!q) return all;
    return all.filter((i) =>
      [i.name, i.id, i.mc_version, i.loader].some((f) => f?.toLowerCase().includes(q)),
    );
  };

  async function install(v: ManifestVersion) {
    setInstalling(v.id);
    setProgress("准备…");
    try {
      await api.installVersion(activeRoot(), v.id);
      toast({ type: "success", message: `已安装 ${v.id}` });
      setShowInstall(false);
      refetch();
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
    } finally {
      setInstalling(null);
      setProgress("");
    }
  }

  return (
    <div class="p-[24px_28px] overflow-y-auto h-full">
      <div class="flex items-center justify-between mb-[18px]">
        <h1 class="text-[24px] font-bold text-fg m-0">库</h1>
        <Button variant="primary" onClick={() => setShowInstall((s) => !s)}>
          {showInstall() ? "关闭" : "安装新版本"}
        </Button>
      </div>

      <Show when={showInstall()}>
        <div class="bg-glass-card rounded-card p-[18px] mb-[20px]">
          <SearchBox
            value={filter()}
            onInput={setFilter}
            placeholder="搜索版本号,如 1.20.1"
          />
          <Show when={installing()}>
            <div class="flex items-center gap-[8px] text-a-5 mt-[12px] text-[13px]">
              <Spinner /> 安装 {installing()} · {progress()}
            </div>
          </Show>
          <Show when={!versions.loading} fallback={<div class="flex justify-center mt-[14px]"><Spinner /></div>}>
            <Show
              when={!versions.error}
              fallback={
                <div class="mt-[14px]">
                  <ErrorState message="版本清单加载失败,请检查网络后重试。" onRetry={() => void refetchVersions()} />
                </div>
              }
            >
              <div class="max-h-[320px] overflow-y-auto mt-[14px] flex flex-col gap-[6px]">
                <For each={filtered()}>
                  {(v) => (
                    <div class="flex items-center gap-[12px] px-[10px] py-[6px] rounded-ctl hover:bg-glass-hover">
                      <span class="font-semibold text-fg min-w-[120px]">{v.id}</span>
                      <span class="text-dim text-[12px] flex-1">{v.kind}</span>
                      <Button
                        variant="ghost"
                        disabled={!!installing()}
                        onClick={() => install(v)}
                      >
                        安装
                      </Button>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </div>
      </Show>

      <Show when={!instances.loading} fallback={<div class="flex justify-center p-[32px]"><Spinner /></div>}>
        <Show
          when={(instances() ?? []).length > 0}
          fallback={<EmptyState title="这个根目录还没有实例,点「安装新版本」开始。" />}
        >
          {/* 实例较多时给个搜索框(≥6 个才显示,避免少量实例时多余 chrome)。 */}
          <Show when={(instances() ?? []).length >= 6}>
            <div class="mb-[12px]">
              <SearchBox
                value={instQuery()}
                onInput={setInstQuery}
                placeholder="搜索实例(名称 / 版本 / 加载器)"
              />
            </div>
          </Show>
          <Show
            when={filteredInstances().length > 0}
            fallback={<EmptyState title={<>没有匹配「{instQuery()}」的实例。</>} />}
          >
          <div class="flex flex-col gap-[10px]">
            <For each={filteredInstances()}>
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
                      refetch();
                  }}
                />
              )}
            </For>
          </div>
          </Show>
        </Show>
      </Show>

    </div>
  );
};

export default Library;
