import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import {
  InstanceRow,
  Button,
  BlockedFilesDialog,
  ImportModpackDialog,
  ExportModpackDialog,
  EmptyState,
  ErrorState,
  Spinner,
  SearchBox,
  toast,
  type InstanceRowData,
} from "../components";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot, openInstance, playInstance } from "../store";
import { openInstanceDir, deleteInstance } from "../util/instanceActions";
import { sortByRecent } from "../util/instances";
import { t } from "../i18n";
import type { InstanceSummary, ManifestVersion, ImportOutcome } from "../ipc/types";

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
  // 导入整合包:统一弹窗(展示支持格式 / 拖入提示 / 须知);产物里有需手动下载或跳过的文件再摊开。
  const [importOpen, setImportOpen] = createSignal(false);
  const [importOutcome, setImportOutcome] = createSignal<ImportOutcome | null>(null);
  // 导出整合包:选格式弹窗(非空 = 打开)。
  const [exportRow, setExportRow] = createSignal<InstanceRowData | null>(null);

  function handleImported(out: ImportOutcome) {
    refetch();
    if (out.blocked.length > 0 || out.skipped_optional.length > 0) setImportOutcome(out);
    else toast({ type: "success", message: t("library.imported", { id: out.instance_id }) });
  }

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
  const sortedInstances = () => sortByRecent(instances() ?? []);

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
    setProgress(t("library.preparing"));
    try {
      await api.installVersion(activeRoot(), v.id);
      toast({ type: "success", message: t("library.installed", { id: v.id }) });
      setShowInstall(false);
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("library.installFailed", { err: String(e) }) });
    } finally {
      setInstalling(null);
      setProgress("");
    }
  }

  return (
    <div class="p-[24px_28px] overflow-y-auto h-full">
      <div class="flex items-center justify-between mb-[18px]">
        <h1 class="text-[24px] font-bold text-fg m-0">{t("library.title")}</h1>
        <div class="flex items-center gap-[10px]">
          <Button variant="ghost" onClick={() => setImportOpen(true)}>
            {t("library.importModpack")}
          </Button>
          <Button variant="primary" onClick={() => setShowInstall((s) => !s)}>
            {showInstall() ? t("library.close") : t("library.installNewVersion")}
          </Button>
        </div>
      </div>

      <Show when={showInstall()}>
        <div class="bg-glass-card rounded-card p-[18px] mb-[20px]">
          <SearchBox
            value={filter()}
            onInput={setFilter}
            placeholder={t("library.searchVersionPlaceholder")}
          />
          <Show when={installing()}>
            <div class="flex items-center gap-[8px] text-a-5 mt-[12px] text-[13px]">
              <Spinner /> {t("library.installingStatus")} {installing()} · {progress()}
            </div>
          </Show>
          <Show when={!versions.loading} fallback={<div class="flex justify-center mt-[14px]"><Spinner /></div>}>
            <Show
              when={!versions.error}
              fallback={
                <div class="mt-[14px]">
                  <ErrorState message={t("library.versionListError")} onRetry={() => void refetchVersions()} />
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
                        {t("library.install")}
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
          fallback={<EmptyState title={t("library.emptyRoot")} />}
        >
          {/* 实例较多时给个搜索框(≥6 个才显示,避免少量实例时多余 chrome)。 */}
          <Show when={(instances() ?? []).length >= 6}>
            <div class="mb-[12px]">
              <SearchBox
                value={instQuery()}
                onInput={setInstQuery}
                placeholder={t("library.searchInstancePlaceholder")}
              />
            </div>
          </Show>
          <Show
            when={filteredInstances().length > 0}
            fallback={<EmptyState title={t("library.noMatch", { query: instQuery() })} />}
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
                  onExport={() => setExportRow(toRowData(inst))}
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

      <ImportModpackDialog
        open={importOpen()}
        root={activeRoot()}
        onClose={() => setImportOpen(false)}
        onImported={handleImported}
      />

      <ExportModpackDialog
        open={!!exportRow()}
        root={activeRoot()}
        instance={exportRow()}
        onClose={() => setExportRow(null)}
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

export default Library;
