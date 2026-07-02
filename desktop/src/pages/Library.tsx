import { useEffect, useState } from "react";
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
import { useAsync } from "../util/useAsync";
import { api, onInstallProgress } from "../ipc/api";
import {
  useAppStore,
  activeRoot,
  openInstance,
  playInstance,
  isRunning,
  refreshInstances,
  checkAllUpdates,
} from "../store";
import { openInstanceDir, deleteInstance } from "../util/instanceActions";
import { sortByRecent } from "../util/instances";
import { t, useLang } from "../i18n";
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

export default function Library() {
  useLang();

  // instances / refreshInstances 来自全局 store(单一真相,见 store.ts)。
  const instances = useAppStore((s) => s.instances);
  const currentRoot = useAppStore((s) => s.currentRoot);
  const checkingUpdates = useAppStore((s) => s.checkingUpdates);
  const updatedInstanceCount = useAppStore((s) => Object.keys(s.updatesByInstance).length);
  const socialEnabled = useAppStore((s) => s.socialEnabled);

  const versions = useAsync(() => api.listVersions(false), []);

  const [showInstall, setShowInstall] = useState(false);
  const [filter, setFilter] = useState("");
  // 实例列表过滤(名称/版本/加载器),与版本安装抽屉的搜索相互独立。
  const [instQuery, setInstQuery] = useState("");
  // 实例列表排序键:recent(默认,上次游玩降序)/ name / version。
  const [sortKey, setSortKey] = useState<"recent" | "name" | "version">("recent");
  // 标签筛选:已选标签集合;空 = 不筛选(显示全部)。OR 语义:命中任一选中标签即保留。
  const [activeTags, setActiveTags] = useState<Set<string>>(new Set());
  const [installing, setInstalling] = useState<string | null>(null);
  const [progress, setProgress] = useState("");
  // 导入整合包:统一弹窗(展示支持格式 / 拖入提示 / 须知);产物里有需手动下载或跳过的文件再摊开。
  const [importOpen, setImportOpen] = useState(false);
  const [joinOpen, setJoinOpen] = useState(false);
  const [importPath, setImportPath] = useState<string | null>(null);
  const [importOutcome, setImportOutcome] = useState<ImportOutcome | null>(null);
  // 导出整合包:选格式弹窗(非空 = 打开)。
  const [exportRow, setExportRow] = useState<InstanceRowData | null>(null);
  // 多选模式 + 已选 id 集合(批量操作,目前为批量删除)。退出选择模式即清空。
  const [selectMode, setSelectMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [bulkConfirm, setBulkConfirm] = useState(false);
  const [bulkDeleting, setBulkDeleting] = useState(false);

  function handleImported(out: ImportOutcome) {
    refreshInstances();
    if (out.blocked.length > 0 || out.skipped_optional.length > 0) setImportOutcome(out);
    else toast({ type: "success", message: t("library.imported", { id: out.instance_id }) });
  }

  // 把整合包文件拖到本页任意处即导入:打开弹窗并自动开始导入(弹窗已开时让它自己接管)。
  const dragOver = useModpackDrop({
    enabled: !importOpen,
    onFile: (path) => {
      setImportPath(path);
      setImportOpen(true);
    },
    onUnsupported: () => toast({ type: "info", message: t("components.import.unsupported") }),
  });

  useEffect(() => {
    return onInstallProgress((p) => {
      if (p.total > 0) setProgress(`${p.stage} ${p.current}/${p.total}`);
      else setProgress(p.stage);
    });
  }, []);

  const filtered = () => {
    const q = filter.toLowerCase();
    return (versions.data ?? []).filter((v) => v.id.toLowerCase().includes(q)).slice(0, 60);
  };

  // 默认按上次游玩降序(最近玩的在前,与首页「继续游玩」同序);未玩过的(0)沉底。
  // 也可切到名称 / 版本升序。
  const sortedInstances = () => {
    const all = instances ?? [];
    switch (sortKey) {
      case "name":
        return [...all].sort((a, b) => (a.name || a.id).localeCompare(b.name || b.id));
      case "version":
        return [...all].sort((a, b) => (a.mc_version || "").localeCompare(b.mc_version || ""));
      default:
        return sortByRecent(all);
    }
  };

  // 当前所有实例上出现过的不同标签(字典序),用于渲染筛选条。
  const allTags = () => {
    const set = new Set<string>();
    for (const i of instances ?? []) for (const tag of i.tags ?? []) set.add(tag);
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
    if (activeTags.size > 0) all = all.filter((i) => (i.tags ?? []).some((tag) => activeTags.has(tag)));
    const q = instQuery.trim().toLowerCase();
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
    return all.length > 0 && all.every((i) => selectedIds.has(i.id));
  };
  function toggleSelectAll() {
    if (allSelected()) setSelectedIds(new Set<string>());
    else setSelectedIds(new Set(filteredInstances().map((i) => i.id)));
  }

  async function bulkDelete() {
    setBulkConfirm(false);
    setBulkDeleting(true);
    const ids = [...selectedIds];
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

  const instList = instances ?? [];

  return (
    <div className="relative p-[24px_28px] overflow-y-auto h-full">
      {dragOver && (
        <div className="absolute inset-0 z-30 flex items-center justify-center bg-[rgba(8,7,5,0.55)] pointer-events-none">
          <Panel
            variant="raised"
            className="flex flex-col items-center gap-[10px] border-2 border-dashed border-accent px-[40px] py-[32px]"
          >
            <Icon name="download" size={30} className="text-accent" />
            <div className="text-[14px] font-medium text-fg">{t("components.import.dropOverlay")}</div>
          </Panel>
        </div>
      )}
      <div className="flex items-center justify-between mb-[18px]">
        <Heading size="page">{t("library.title")}</Heading>
        <div className="flex items-center gap-[10px]">
          {instList.length > 0 && (
            <Button variant="ghost" onClick={() => (selectMode ? exitSelect() : setSelectMode(true))}>
              {selectMode ? t("library.selectDone") : t("library.select")}
            </Button>
          )}
          {!selectMode && (
            <>
              {/* 一键检查所有实例的更新(按需触发,绝不自动跑);完成后用 N 摘要 + 卡片角标提示。 */}
              {instList.length > 0 && (
                <>
                  {!checkingUpdates && updatedInstanceCount > 0 && (
                    <span className="text-[12px] text-accent">
                      {t("library.updatesSummary", { n: updatedInstanceCount })}
                    </span>
                  )}
                  <Button variant="ghost" disabled={checkingUpdates} onClick={() => void checkAllUpdates()}>
                    {checkingUpdates ? (
                      <span className="flex items-center gap-[6px]">
                        <Spinner /> {t("library.checkingUpdates")}
                      </span>
                    ) : (
                      t("library.checkUpdates")
                    )}
                  </Button>
                </>
              )}
              {socialEnabled && (
                <Button variant="ghost" onClick={() => setJoinOpen(true)}>
                  {t("realm.joinAction")}
                </Button>
              )}
              <Button variant="ghost" onClick={() => setImportOpen(true)}>
                {t("library.importModpack")}
              </Button>
              <Button variant="primary" onClick={() => setShowInstall((s) => !s)}>
                {showInstall ? t("library.close") : t("library.installNewVersion")}
              </Button>
            </>
          )}
        </div>
      </div>

      {/* 批量操作条:仅多选模式可见。展示已选数 + 全选/清空 + 删除所选。 */}
      {selectMode && (
        <Panel variant="raised" className="flex items-center gap-[12px] mb-[16px] px-[14px] py-[10px]">
          <span className="text-[13px] font-medium text-fg">
            {t("library.selectedCount", { count: selectedIds.size })}
          </span>
          <Button variant="ghost" onClick={toggleSelectAll}>
            {allSelected() ? t("library.clearSelection") : t("library.selectAll")}
          </Button>
          <div className="flex-1" />
          <Button
            variant="danger"
            disabled={selectedIds.size === 0 || bulkDeleting}
            onClick={() => setBulkConfirm(true)}
          >
            {t("library.deleteSelected")}
          </Button>
        </Panel>
      )}

      {showInstall && (
        <Panel variant="sunken" className="p-[18px] mb-[20px]">
          <SearchBox
            value={filter}
            onInput={setFilter}
            placeholder={t("library.searchVersionPlaceholder")}
          />
          {installing && (
            <div className="flex items-center gap-[8px] text-accent mt-[12px] text-[13px]">
              <Spinner /> {t("library.installingStatus")} {installing} · {progress}
            </div>
          )}
          {versions.loading ? (
            <div className="flex justify-center mt-[14px]"><Spinner /></div>
          ) : versions.error ? (
            <div className="mt-[14px]">
              <ErrorState message={t("library.versionListError")} onRetry={() => versions.refetch()} />
            </div>
          ) : (
            <div className="max-h-[320px] overflow-y-auto mt-[14px] flex flex-col gap-[6px]">
              {filtered().map((v) => (
                <div
                  key={v.id}
                  className="flex items-center gap-[12px] px-[10px] py-[6px] hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app"
                >
                  <span className="font-pixel text-[11px] text-fg min-w-[120px]">{v.id}</span>
                  <span className="text-muted text-[12px] flex-1">{v.kind}</span>
                  <Button variant="ghost" disabled={!!installing} onClick={() => install(v)}>
                    {t("library.install")}
                  </Button>
                </div>
              ))}
            </div>
          )}
        </Panel>
      )}

      {instances === undefined ? (
        <div className="flex justify-center p-[32px]"><Spinner /></div>
      ) : instList.length > 0 ? (
        <>
          {/* 标签筛选条:任意实例带标签时出现。「全部」清空,其余 chip OR 多选。 */}
          {allTags().length > 0 && (
            <div className="flex flex-wrap items-center gap-[8px] mb-[12px]">
              <span className="text-[12px] text-muted shrink-0">{t("tags.filterLabel")}</span>
              <Chip active={activeTags.size === 0} onClick={() => setActiveTags(new Set())}>
                {t("tags.filterAll")}
              </Chip>
              {allTags().map((tag) => (
                <Chip key={tag} active={activeTags.has(tag)} onClick={() => toggleTag(tag)}>
                  {tag}
                </Chip>
              ))}
            </div>
          )}
          {/* 实例较多时给搜索框 + 排序(≥6 个才显示,避免少量实例时多余 chrome)。 */}
          {instList.length >= 6 && (
            <div className="flex items-center gap-[10px] mb-[12px]">
              <div className="flex-1 min-w-0">
                <SearchBox
                  value={instQuery}
                  onInput={setInstQuery}
                  placeholder={t("library.searchInstancePlaceholder")}
                />
              </div>
              <Select
                value={sortKey}
                onChange={(v) => setSortKey(v as "recent" | "name" | "version")}
                className="min-w-[150px] shrink-0"
                options={[
                  { value: "recent", label: t("library.sortRecent") },
                  { value: "name", label: t("library.sortName") },
                  { value: "version", label: t("library.sortVersion") },
                ]}
              />
            </div>
          )}
          {filteredInstances().length > 0 ? (
            <div className="flex flex-col gap-[10px]">
              {filteredInstances().map((inst) => (
                <InstanceRow
                  key={inst.id}
                  instance={toRowData(inst)}
                  selectable={selectMode}
                  selected={selectedIds.has(inst.id)}
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
              ))}
            </div>
          ) : (
            <EmptyState title={t("library.noMatch", { query: instQuery })} />
          )}
        </>
      ) : (
        <EmptyState title={t("library.emptyRoot")} />
      )}

      <ImportModpackDialog
        open={importOpen}
        root={currentRoot ?? ""}
        initialPath={importPath}
        onClose={() => {
          setImportOpen(false);
          setImportPath(null);
        }}
        onImported={handleImported}
      />

      <ExportModpackDialog
        open={!!exportRow}
        root={currentRoot ?? ""}
        instance={exportRow}
        onClose={() => setExportRow(null)}
      />

      <JoinRealmDialog
        open={joinOpen}
        onClose={() => setJoinOpen(false)}
        onJoined={(instanceId) => {
          setJoinOpen(false);
          openInstance(instanceId);
        }}
      />

      {/* 批量删除确认。 */}
      <Dialog
        open={bulkConfirm}
        onClose={() => setBulkConfirm(false)}
        label={t("library.bulkDeleteTitle")}
        contentClass="w-[380px] max-w-[calc(100vw-48px)] bg-panel text-fg border border-titlebar shadow-raised rounded-none overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <Heading size="sub">{t("library.bulkDeleteTitle")}</Heading>
          <div className="text-[13px] text-sub leading-[1.6]">
            {t("library.bulkDeleteBody", { count: selectedIds.size })}
          </div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setBulkConfirm(false)}>
              {t("instance.cancel")}
            </Button>
            <Button variant="danger" onClick={bulkDelete}>
              {t("library.deleteSelected")}
            </Button>
          </div>
        </div>
      </Dialog>

      {importOutcome && (
        <BlockedFilesDialog
          instanceId={importOutcome.instance_id}
          blocked={importOutcome.blocked}
          skipped={importOutcome.skipped_optional}
          onClose={() => setImportOutcome(null)}
        />
      )}
    </div>
  );
}
