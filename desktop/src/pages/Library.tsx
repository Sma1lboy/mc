import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import {
  InstanceRow,
  Button,
  Chip,
  Panel,
  Heading,
  Select,
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
import { JoinRealmDialog } from "../components/JoinRealmDialog";
import { useModpackDrop } from "../util/useModpackDrop";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot, openInstance, playInstance, isRunning, instances, refreshInstances, socialEnabled } from "../store";
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
    realmRole: i.realm?.role,
    installed: i.installed,
    tags: i.tags ?? [],
  };
}

const Library: Component = () => {
  // instances / refreshInstances 来自全局 store(单一真相,见 store.ts)。
  const [versions, { refetch: refetchVersions }] = createResource(() => api.listVersions(false));

  const [showInstall, setShowInstall] = createSignal(false);
  const [filter, setFilter] = createSignal("");
  // 实例列表过滤(名称/版本/加载器),与版本安装抽屉的搜索相互独立。
  const [instQuery, setInstQuery] = createSignal("");
  // 实例列表排序键:recent(默认,上次游玩降序)/ name / version。
  const [sortKey, setSortKey] = createSignal<"recent" | "name" | "version">("recent");
  // 标签筛选:已选标签集合;空 = 不筛选(显示全部)。OR 语义:命中任一选中标签即保留。
  const [activeTags, setActiveTags] = createSignal<Set<string>>(new Set());
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [progress, setProgress] = createSignal("");
  // 导入整合包:统一弹窗(展示支持格式 / 拖入提示 / 须知);产物里有需手动下载或跳过的文件再摊开。
  const [importOpen, setImportOpen] = createSignal(false);
  const [joinOpen, setJoinOpen] = createSignal(false);
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
  // 也可切到名称 / 版本升序。
  const sortedInstances = () => {
    const all = instances() ?? [];
    switch (sortKey()) {
      case "name":
        return [...all].sort((a, b) =>
          (a.name || a.id).localeCompare(b.name || b.id),
        );
      case "version":
        return [...all].sort((a, b) => (a.mc_version || "").localeCompare(b.mc_version || ""));
      default:
        return sortByRecent(all);
    }
  };

  // 当前所有实例上出现过的不同标签(字典序),用于渲染筛选条。
  const allTags = () => {
    const set = new Set<string>();
    for (const i of instances() ?? []) for (const tag of i.tags ?? []) set.add(tag);
    return [...set].sort((a, b) => a.localeCompare(b));
  };

  function toggleTag(tag: string) {
    setActiveTags((s) => {
      const n = new Set(s);
      if (n.has(tag)) n.delete(tag);
      else n.add(tag);
      return n;
    });
  }

  // 在排序基础上按名称 / 版本 / 加载器过滤 + 标签筛选(空查询/空标签集返回全部)。
  const filteredInstances = () => {
    let all = sortedInstances();
    const active = activeTags();
    // 标签 OR:实例命中任一选中标签即保留。
    if (active.size > 0) all = all.filter((i) => (i.tags ?? []).some((tag) => active.has(tag)));
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
        <div class="absolute inset-0 z-30 flex items-center justify-center bg-[rgba(8,7,5,0.55)] pointer-events-none">
          <Panel
            variant="raised"
            class="flex flex-col items-center gap-[10px] border-2 border-dashed border-accent px-[40px] py-[32px]"
          >
            <Icon name="download" size={30} class="text-accent" />
            <div class="text-[14px] font-medium text-fg">{t("components.import.dropOverlay")}</div>
          </Panel>
        </div>
      </Show>
      <div class="flex items-center justify-between mb-[18px]">
        <Heading size="page">{t("library.title")}</Heading>
        <div class="flex items-center gap-[10px]">
          <Show when={(instances() ?? []).length > 0}>
            <Button variant="ghost" onClick={() => (selectMode() ? exitSelect() : setSelectMode(true))}>
              {selectMode() ? t("library.selectDone") : t("library.select")}
            </Button>
          </Show>
          <Show when={!selectMode()}>
            <Show when={socialEnabled()}>
              <Button variant="ghost" onClick={() => setJoinOpen(true)}>
                {t("realm.joinAction")}
              </Button>
            </Show>
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
        <Panel variant="raised" class="flex items-center gap-[12px] mb-[16px] px-[14px] py-[10px]">
          <span class="text-[13px] font-medium text-fg">
            {t("library.selectedCount", { count: selectedIds().size })}
          </span>
          <Button variant="ghost" onClick={toggleSelectAll}>
            {allSelected() ? t("library.clearSelection") : t("library.selectAll")}
          </Button>
          <div class="flex-1" />
          <Button
            variant="danger"
            disabled={selectedIds().size === 0 || bulkDeleting()}
            onClick={() => setBulkConfirm(true)}
          >
            {t("library.deleteSelected")}
          </Button>
        </Panel>
      </Show>

      <Show when={showInstall()}>
        <Panel variant="sunken" class="p-[18px] mb-[20px]">
          <SearchBox
            value={filter()}
            onInput={setFilter}
            placeholder={t("library.searchVersionPlaceholder")}
          />
          <Show when={installing()}>
            <div class="flex items-center gap-[8px] text-accent mt-[12px] text-[13px]">
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
                    <div class="flex items-center gap-[12px] px-[10px] py-[6px] hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app">
                      <span class="font-pixel text-[11px] text-fg min-w-[120px]">{v.id}</span>
                      <span class="text-muted text-[12px] flex-1">{v.kind}</span>
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
        </Panel>
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
          {/* 标签筛选条:任意实例带标签时出现。「全部」清空,其余 chip OR 多选。 */}
          <Show when={allTags().length > 0}>
            <div class="flex flex-wrap items-center gap-[8px] mb-[12px]">
              <span class="text-[12px] text-muted shrink-0">{t("tags.filterLabel")}</span>
              <Chip active={activeTags().size === 0} onClick={() => setActiveTags(new Set())}>
                {t("tags.filterAll")}
              </Chip>
              <For each={allTags()}>
                {(tag) => (
                  <Chip active={activeTags().has(tag)} onClick={() => toggleTag(tag)}>
                    {tag}
                  </Chip>
                )}
              </For>
            </div>
          </Show>
          {/* 实例较多时给搜索框 + 排序(≥6 个才显示,避免少量实例时多余 chrome)。 */}
          <Show when={(instances() ?? []).length >= 6}>
            <div class="flex items-center gap-[10px] mb-[12px]">
              <div class="flex-1 min-w-0">
                <SearchBox
                  value={instQuery()}
                  onInput={setInstQuery}
                  placeholder={t("library.searchInstancePlaceholder")}
                />
              </div>
              <Select
                value={sortKey()}
                onChange={(v) => setSortKey(v as "recent" | "name" | "version")}
                class="min-w-[150px] shrink-0"
                options={[
                  { value: "recent", label: t("library.sortRecent") },
                  { value: "name", label: t("library.sortName") },
                  { value: "version", label: t("library.sortVersion") },
                ]}
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

      <JoinRealmDialog
        open={joinOpen()}
        onClose={() => setJoinOpen(false)}
        onJoined={(instanceId) => {
          setJoinOpen(false);
          openInstance(instanceId);
        }}
      />

      {/* 批量删除确认。 */}
      <Dialog
        open={bulkConfirm()}
        onClose={() => setBulkConfirm(false)}
        label={t("library.bulkDeleteTitle")}
        contentClass="w-[380px] max-w-[calc(100vw-48px)] bg-panel text-fg border border-titlebar shadow-raised rounded-none overflow-hidden"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <Heading size="sub">{t("library.bulkDeleteTitle")}</Heading>
          <div class="text-[13px] text-sub leading-[1.6]">
            {t("library.bulkDeleteBody", { count: selectedIds().size })}
          </div>
          <div class="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setBulkConfirm(false)}>
              {t("instance.cancel")}
            </Button>
            <Button variant="danger" onClick={bulkDelete}>
              {t("library.deleteSelected")}
            </Button>
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
