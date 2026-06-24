import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import {
  InstanceRow,
  Button,
  BlockedFilesDialog,
  ImportModpackDialog,
  ExportModpackDialog,
  Dialog,
  EmptyState,
  ErrorState,
  Icon,
  Spinner,
  SearchBox,
  toast,
  type InstanceRowData,
} from "../components";
import { useModpackDrop } from "../util/useModpackDrop";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot, openInstance, playInstance, isRunning, instances, refreshInstances } from "../store";
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
  // instances / refreshInstances 来自全局 store(单一真相,见 store.ts)。
  const [versions, { refetch: refetchVersions }] = createResource(() => api.listVersions(false));

  const [showInstall, setShowInstall] = createSignal(false);
  const [filter, setFilter] = createSignal("");
  // 实例列表过滤(名称/版本/加载器),与版本安装抽屉的搜索相互独立。
  const [instQuery, setInstQuery] = createSignal("");
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [progress, setProgress] = createSignal("");
  // 导入整合包:统一弹窗(展示支持格式 / 拖入提示 / 须知);产物里有需手动下载或跳过的文件再摊开。
  const [importOpen, setImportOpen] = createSignal(false);
  const [importPath, setImportPath] = createSignal<string | null>(null);
  const [importOutcome, setImportOutcome] = createSignal<ImportOutcome | null>(null);
  // 导出整合包:选格式弹窗(非空 = 打开)。
  const [exportRow, setExportRow] = createSignal<InstanceRowData | null>(null);
  // 多选模式 + 已选 id 集合(批量操作,目前为批量删除)。退出选择模式即清空。
  const [selectMode, setSelectMode] = createSignal(false);
  const [selectedIds, setSelectedIds] = createSignal<Set<string>>(new Set());
  const [bulkConfirm, setBulkConfirm] = createSignal(false);
  const [bulkDeleting, setBulkDeleting] = createSignal(false);

  function handleImported(out: ImportOutcome) {
    refreshInstances();
    if (out.blocked.length > 0 || out.skipped_optional.length > 0) setImportOutcome(out);
    else toast({ type: "success", message: t("library.imported", { id: out.instance_id }) });
  }

  // 把整合包文件拖到本页任意处即导入:打开弹窗并自动开始导入(弹窗已开时让它自己接管)。
  const dragOver = useModpackDrop({
    enabled: () => !importOpen(),
    onFile: (path) => {
      setImportPath(path);
      setImportOpen(true);
    },
    onUnsupported: () => toast({ type: "info", message: t("components.import.unsupported") }),
  });

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
      refreshInstances();
    } catch (e) {
      toast({ type: "error", message: t("library.installFailed", { err: String(e) }) });
    } finally {
      setInstalling(null);
      setProgress("");
    }
  }

  // ===== 多选 / 批量操作 =====
  function toggleSelect(id: string) {
    setSelectedIds((s) => {
      const n = new Set(s);
      if (n.has(id)) n.delete(id);
      else n.add(id);
      return n;
    });
  }
  function exitSelect() {
    setSelectMode(false);
    setSelectedIds(new Set<string>());
  }
  // 当前过滤结果是否已全选(空列表视为未全选)。
  const allSelected = () => {
    const all = filteredInstances();
    return all.length > 0 && all.every((i) => selectedIds().has(i.id));
  };
  function toggleSelectAll() {
    if (allSelected()) setSelectedIds(new Set<string>());
    else setSelectedIds(new Set(filteredInstances().map((i) => i.id)));
  }

  async function bulkDelete() {
    setBulkConfirm(false);
    setBulkDeleting(true);
    const ids = [...selectedIds()];
    const root = activeRoot();
    let ok = 0;
    let skipped = 0;
    let failed = 0;
    for (const id of ids) {
      // 运行中的实例不能删(游戏占着目录文件),整批里直接跳过并计数。
      if (isRunning(id)) {
        skipped++;
        continue;
      }
      try {
        await api.deleteInstance(root, id);
        ok++;
      } catch {
        failed++;
      }
    }
    if (ok > 0) toast({ type: "success", message: t("library.bulkDeleted", { count: ok }) });
    if (skipped > 0) toast({ type: "info", message: t("library.bulkDeleteRunningSkipped", { count: skipped }) });
    if (failed > 0) toast({ type: "error", message: t("library.bulkDeleteFailed", { count: failed }) });
    setBulkDeleting(false);
    exitSelect();
    refreshInstances();
  }

  return (
    <div class="relative p-[24px_28px] overflow-y-auto h-full">
      <Show when={dragOver()}>
        <div class="absolute inset-0 z-30 flex items-center justify-center bg-black/40 backdrop-blur-sm pointer-events-none">
          <div class="flex flex-col items-center gap-[10px] rounded-card border-2 border-dashed border-a-4 bg-glass-card px-[40px] py-[32px]">
            <Icon name="download" size={30} class="text-a-5" />
            <div class="text-[14px] font-semibold text-fg">{t("components.import.dropOverlay")}</div>
          </div>
        </div>
      </Show>
      <div class="flex items-center justify-between mb-[18px]">
        <h1 class="text-[24px] font-bold text-fg m-0">{t("library.title")}</h1>
        <div class="flex items-center gap-[10px]">
          <Show when={(instances() ?? []).length > 0}>
            <Button variant="ghost" onClick={() => (selectMode() ? exitSelect() : setSelectMode(true))}>
              {selectMode() ? t("library.selectDone") : t("library.select")}
            </Button>
          </Show>
          <Show when={!selectMode()}>
            <Button variant="ghost" onClick={() => setImportOpen(true)}>
              {t("library.importModpack")}
            </Button>
            <Button variant="primary" onClick={() => setShowInstall((s) => !s)}>
              {showInstall() ? t("library.close") : t("library.installNewVersion")}
            </Button>
          </Show>
        </div>
      </div>

      {/* 批量操作条:仅多选模式可见。展示已选数 + 全选/清空 + 删除所选。 */}
      <Show when={selectMode()}>
        <div class="flex items-center gap-[12px] mb-[16px] px-[14px] py-[10px] rounded-card glass-card">
          <span class="text-[13px] font-medium text-fg">
            {t("library.selectedCount", { count: selectedIds().size })}
          </span>
          <button
            type="button"
            class="h-[30px] px-[12px] rounded-ctl border border-glass-border bg-glass-card text-fg text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-glass-hover"
            onClick={toggleSelectAll}
          >
            {allSelected() ? t("library.clearSelection") : t("library.selectAll")}
          </button>
          <div class="flex-1" />
          <button
            type="button"
            class="h-[34px] px-[16px] rounded-ctl border-none bg-danger text-white text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-danger-hover disabled:opacity-50 disabled:cursor-not-allowed"
            disabled={selectedIds().size === 0 || bulkDeleting()}
            onClick={() => setBulkConfirm(true)}
          >
            {t("library.deleteSelected")}
          </button>
        </div>
      </Show>

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
          when={!instances.error}
          fallback={<ErrorState message={t("library.instanceListError")} onRetry={() => void refreshInstances()} />}
        >
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
                  selectable={selectMode()}
                  selected={selectedIds().has(inst.id)}
                  onToggleSelect={toggleSelect}
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
      </Show>

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

      <ExportModpackDialog
        open={!!exportRow()}
        root={activeRoot()}
        instance={exportRow()}
        onClose={() => setExportRow(null)}
      />

      {/* 批量删除确认。 */}
      <Dialog
        open={bulkConfirm()}
        onClose={() => setBulkConfirm(false)}
        label={t("library.bulkDeleteTitle")}
        contentClass="w-[380px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-fg">{t("library.bulkDeleteTitle")}</div>
          <div class="text-[13px] text-dim leading-[1.6]">
            {t("library.bulkDeleteBody", { count: selectedIds().size })}
          </div>
          <div class="flex justify-end gap-[10px]">
            <button
              class="h-[34px] px-[16px] border border-glass-border rounded-ctl bg-glass-card text-fg text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-glass-hover"
              onClick={() => setBulkConfirm(false)}
            >
              {t("instance.cancel")}
            </button>
            <button
              class="h-[34px] px-[16px] border-none rounded-ctl bg-danger text-white text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-danger-hover"
              onClick={bulkDelete}
            >
              {t("library.deleteSelected")}
            </button>
          </div>
        </div>
      </Dialog>

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
