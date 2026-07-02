import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import clsx from "clsx";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Dialog } from "./Dialog";
import { InstanceIcon } from "./InstanceIcon";
import Lightbox from "./Lightbox";
import ServersPanel from "./ServersPanel";
import { RealmPanel } from "./RealmPanel";
import { ContentBrowser, type ContentProvider } from "./ContentBrowser";
import { ErrorState } from "./ErrorState";
import { ACCENT_BTN_COMPACT, ACCENT_BTN } from "./styles";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { Toggle } from "./Toggle";
import { ModpackOverview } from "./ModpackOverview";
import type { ModpackHit } from "./ModpackCard";
import ProjectInstallDetail from "../pages/ProjectInstallDetail";
import { Spinner } from "./Spinner";
import { Select } from "./Select";
import { toast } from "./Toast";
import { api, onInstallProgress } from "../ipc/api";
import { openInstanceDir, openInstanceSubdir } from "../util/instanceActions";
import { activeRoot, openInstance, isRunning, useAppStore } from "../store";
import { useAsync } from "../util/useAsync";
import { t, useLang } from "../i18n";
import type {
  InstanceConfig,
  InstanceSummary,
  ModInfo,
  ModUpdate,
  PackKind,
  PackInfo,
  ProjectKind,
  WorldInfo,
  ScreenshotInfo,
} from "../ipc/types";

/**
 * InstanceManageDialog —— 单实例管理:设置(名字/内存/Java/JVM/窗口)+ Mods(启停/删除)。
 * 设置改一项即 set_instance_config 持久化;Mods 用 set_mod_enabled / delete_mod。
 */

const FIELD =
  "bg-sidebar shadow-input h-[34px] px-[12px] rounded-none text-fg text-[13px] " +
  "placeholder:text-faint transition-[box-shadow] duration-150 focus-visible:outline-none " +
  "focus-visible:ring-2 focus-visible:ring-accent";
const LABEL = "text-[12px] text-sub";
const TAB =
  "px-[14px] py-[7px] text-[13px] font-semibold cursor-pointer border-b-2 border-b-transparent " +
  "text-muted hover:text-fg transition-colors duration-150";
const TAB_ACTIVE = "!text-accent !border-b-accent";

export type InstanceManageTab =
  | "realm"
  | "overview"
  | "settings"
  | "mods"
  | "resource_pack"
  | "shader"
  | "datapack"
  | "worlds"
  | "servers"
  | "screenshots";

const TABS = (): { key: InstanceManageTab; label: string }[] => [
  { key: "settings", label: t("instance.tabSettings") },
  { key: "mods", label: t("instance.tabMods") },
  { key: "resource_pack", label: t("instance.tabResourcePack") },
  { key: "shader", label: t("instance.tabShader") },
  { key: "datapack", label: t("instance.tabDatapack") },
  { key: "worlds", label: t("instance.tabWorlds") },
  { key: "servers", label: t("instance.tabServers") },
  { key: "screenshots", label: t("instance.tabScreenshots") },
];

const isPackTab = (tab: InstanceManageTab): tab is PackKind =>
  tab === "resource_pack" || tab === "shader" || tab === "datapack";

/** 截图栅格上限:只展示最新 N 张,避免一次性加载海量大图。 */
const SCREENSHOT_CAP = 60;

/**
 * ScreenshotTile —— 单张截图缩略图。用 IntersectionObserver 懒加载:滚动到视口附近才
 * 通过 read_screenshot 取该张的 data URL,避免把目录里所有大图一次性读进内存。
 */
function ScreenshotTile(props: {
  info: ScreenshotInfo;
  url?: string;
  failed?: boolean;
  onVisible: () => void;
  onOpen: () => void;
  onRetry: () => void;
  onDelete: (e: ReactMouseEvent) => void;
}) {
  const elRef = useRef<HTMLDivElement>(null);
  // onVisible 通过 ref 读最新闭包(观察器只装一次,回调却按当前渲染取值)。
  const onVisibleRef = useRef(props.onVisible);
  onVisibleRef.current = props.onVisible;
  useEffect(() => {
    const el = elRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          onVisibleRef.current();
          io.disconnect();
        }
      },
      { rootMargin: "120px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);

  return (
    <div
      ref={elRef}
      className="group relative aspect-video rounded-none overflow-hidden bg-panel-2 cursor-pointer"
      onClick={props.onOpen}
    >
      {props.url ? (
        <img src={props.url} alt={props.info.file_name} width="320" height="180" className="w-full h-full object-cover" />
      ) : props.failed ? (
        // 读图失败:给可重试的占位,而不是永远转圈。
        <button
          className="w-full h-full grid place-items-center gap-[2px] text-[11px] text-muted bg-panel-2 cursor-pointer hover:text-fg"
          onClick={(e) => {
            e.stopPropagation();
            props.onRetry();
          }}
          title={t("instance.reload")}
        >
          <span>{t("instance.loadFailed")}</span>
          <span className="text-[10px] underline">{t("instance.clickRetry")}</span>
        </button>
      ) : (
        <div className="w-full h-full grid place-items-center">
          <Spinner size={16} />
        </div>
      )}
      <button
        className="absolute top-[4px] right-[4px] opacity-0 group-hover:opacity-100 transition-opacity duration-150 text-[11px] text-white px-[6px] py-[2px] rounded-none bg-[rgba(0,0,0,0.55)] hover:bg-danger"
        onClick={props.onDelete}
      >
        {t("instance.delete")}
      </button>
    </div>
  );
}

/**
 * ScreenshotsPanel —— 实例截图栅格:懒加载缩略图、点开进灯箱、悬停删除。
 * 列表只取元数据,图片字节按需 read_screenshot;最多展示 SCREENSHOT_CAP 张(更多时提示)。
 */
function ScreenshotsPanel(props: { instance: InstanceSummary }) {
  const { data: shots, loading: shotsLoading, error: shotsError, refetch } = useAsync<ScreenshotInfo[]>(
    () => api.instanceScreenshots(activeRoot(), props.instance.id),
    [props.instance.id],
  );
  const capped = (shots ?? []).slice(0, SCREENSHOT_CAP);
  const [urls, setUrls] = useState<Record<string, string>>({});
  const [failed, setFailed] = useState<Record<string, boolean>>({});
  const [lightbox, setLightbox] = useState<number | null>(null);
  // loadUrl 的去重要读最新已加载 urls(否则拿到旧闭包会重复取同一张)。
  const urlsRef = useRef(urls);
  urlsRef.current = urls;

  async function loadUrl(fileName: string) {
    if (urlsRef.current[fileName]) return;
    setFailed((f) => ({ ...f, [fileName]: false }));
    try {
      const u = await api.readScreenshot(activeRoot(), props.instance.id, fileName);
      setUrls((m) => ({ ...m, [fileName]: u }));
    } catch {
      // 单张读失败不致命:标记失败态,渲染可重试占位,不让缩略图永远转圈。
      setFailed((f) => ({ ...f, [fileName]: true }));
    }
  }

  async function remove(s: ScreenshotInfo, e: ReactMouseEvent) {
    e.stopPropagation(); // 别触发打开灯箱。
    try {
      await api.deleteScreenshot(activeRoot(), props.instance.id, s.file_name);
      toast({ type: "success", message: t("instance.deletedScreenshot") });
      refetch();
    } catch (err) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(err) }) });
    }
  }

  const lightboxImages = capped.map((s) => ({ url: urls[s.file_name] ?? "", title: s.file_name }));

  // 打开/切换灯箱时确保目标图及左右相邻图已加载(缩略图可能还没滚动到、未触发懒加载),
  // 避免主图/缩略图条出现空白或裂图。
  function openLightbox(i: number) {
    for (const j of [i, i - 1, i + 1]) {
      const f = capped[j]?.file_name;
      if (f) void loadUrl(f);
    }
    setLightbox(i);
  }

  return (
    <div className="flex flex-col gap-[8px]">
      <div className="flex items-center justify-between">
        <div className={LABEL}>{t("instance.screenshots")}</div>
        <button
          className={OPEN_BTN}
          onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "screenshots")}
        >
          {t("instance.openDir")}
        </button>
      </div>

      {(shots ?? []).length > SCREENSHOT_CAP && (
        <div className="text-[11px] text-muted">
          {t("instance.screenshotCapNote", { total: (shots ?? []).length, cap: SCREENSHOT_CAP })}
        </div>
      )}

      {shotsLoading ? (
        <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
          <Spinner size={16} /> {t("instance.scanningScreenshots")}
        </div>
      ) : capped.length > 0 ? (
        <div className="grid grid-cols-3 gap-[8px]">
          {capped.map((s, i) => (
            <ScreenshotTile
              key={s.file_name}
              info={s}
              url={urls[s.file_name]}
              failed={failed[s.file_name]}
              onVisible={() => loadUrl(s.file_name)}
              onOpen={() => openLightbox(i)}
              onRetry={() => loadUrl(s.file_name)}
              onDelete={(e) => remove(s, e)}
            />
          ))}
        </div>
      ) : shotsError ? (
        <ErrorState compact message={t("instance.screenshotLoadError")} onRetry={() => void refetch()} />
      ) : (
        <div className="text-muted text-[13px] py-[12px]">{t("instance.noScreenshots")}</div>
      )}

      {lightbox !== null && (
        <Lightbox
          images={lightboxImages}
          index={lightbox}
          onIndex={openLightbox}
          onClose={() => setLightbox(null)}
        />
      )}
    </div>
  );
}

/** 人类可读的字节大小;0 / 缺省返回空串。 */
function fmtSize(bytes: number): string {
  if (!bytes) return "";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let n = bytes;
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i += 1;
  }
  return `${n.toFixed(i > 0 && n < 10 ? 1 : 0)} ${units[i]}`;
}

const INSTALL_BTN = ACCENT_BTN_COMPACT;
const DEL_BTN =
  "shrink-0 text-[12px] text-danger-text px-[8px] py-[4px] rounded-none cursor-pointer hover:bg-danger-soft";
const OPEN_BTN =
  "shrink-0 text-[12px] text-muted px-[8px] py-[3px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2";

/**
 * PacksPanel —— 资源包 / 光影的统一面板:Modrinth 搜索安装 + 本地启停/删除。
 * 与 Mods 面板同构,差异仅在 PackKind / 搜索资源类型;按本实例 mc 版本过滤,
 * 资源包/光影在 Modrinth 上不按加载器细分,故 loader 传 null。
 */
function PacksPanel(props: {
  instance: InstanceSummary;
  kind: PackKind;
  searchKind: ProjectKind;
  emptyHint: string;
  /** 外部导入计数:递增即触发重扫(拖拽导入后由父组件 bump)。 */
  tick?: number;
  /** 受控的「浏览/添加」模式(由父组件统一持有,用于隐藏详情页头部)。 */
  browse: boolean;
  onBrowse: (v: boolean) => void;
}) {
  // 数据包逐存档生效:落到 saves/<world>/datapacks。其它包类型无 world 概念。
  const isDatapack = props.kind === "datapack";
  const { data: worlds } = useAsync<WorldInfo[] | undefined>(
    () => (isDatapack ? api.instanceWorlds(activeRoot(), props.instance.id) : Promise.resolve(undefined)),
    [isDatapack, props.instance.id],
  );
  const [world, setWorld] = useState<string | null>(null);
  // 默认选中第一个存档(按上次游玩排序);存档变化后若当前选中已失效则回退。
  useEffect(() => {
    if (!isDatapack) return;
    const list = worlds ?? [];
    setWorld((prev) => {
      if (list.length === 0) return null;
      if (!prev || !list.some((w) => w.folder === prev)) return list[0].folder;
      return prev;
    });
  }, [isDatapack, worlds]);
  const worldArg = isDatapack ? world : null;

  const { data: packs, loading: packsLoading, error: packsError, refetch } = useAsync<PackInfo[]>(
    () => api.instancePacks(activeRoot(), props.instance.id, props.kind, worldArg),
    [props.instance.id, props.kind, props.tick ?? 0, worldArg],
  );

  const [detail, setDetail] = useState<ModpackHit | null>(null);
  // 详情页对应的来源平台(随 onOpenDetail 一起带过来),决定详情里取版本/安装走哪个 provider。
  const [detailProvider, setDetailProvider] = useState<ContentProvider>("modrinth");
  // 后台并行安装:正在安装的 project_id 集合(不阻塞其它行)。
  const [installing, setInstalling] = useState<Set<string>>(new Set());
  // 本次浏览已添加的 project_id:行按钮即时变「已添加」。
  const [added, setAdded] = useState<Set<string>>(new Set());
  // 删除资源包前确认(删除是破坏性的,与存档删除一致)。
  const [confirmDel, setConfirmDel] = useState<PackInfo | null>(null);
  const startBrowse = () => {
    setAdded(new Set<string>());
    props.onBrowse(true);
  };

  // 行内「下载」:直接装最新兼容版(资源包/光影/数据包不分加载器),后台并行不阻塞其它行。
  async function install(projectId: string, title: string, provider: ContentProvider = "modrinth") {
    if (installing.has(projectId)) return;
    setInstalling((s) => new Set(s).add(projectId));
    try {
      const report = await api.installPack(
        activeRoot(),
        props.instance.id,
        props.kind,
        projectId,
        props.instance.mc_version,
        worldArg,
        provider,
      );
      if ((report.blocked?.length ?? 0) > 0) {
        toast({ type: "warn", message: t("instance.blockedManual", { count: report.blocked!.length }) });
      } else {
        toast({ type: "success", message: t("instance.installed", { title, file: report.file }) });
        setAdded((s) => new Set(s).add(projectId));
        refetch();
      }
    } catch (e) {
      toast({ type: "error", message: t("instance.installFailed", { err: String(e) }) });
    } finally {
      setInstalling((s) => {
        const n = new Set(s);
        n.delete(projectId);
        return n;
      });
    }
  }

  async function toggle(p: PackInfo, enabled: boolean) {
    try {
      await api.setPackEnabled(activeRoot(), props.instance.id, props.kind, p.file_name, enabled, worldArg);
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.opFailed", { err: String(e) }) });
    }
  }

  async function remove(p: PackInfo) {
    try {
      await api.deletePack(activeRoot(), props.instance.id, props.kind, p.file_name, worldArg);
      toast({ type: "success", message: t("instance.deletedFile", { file: p.file_name }) });
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(e) }) });
    }
  }

  return (
    <div className="flex flex-col gap-[8px]">
      {/* 数据包目标存档选择器:数据包是逐存档生效的,必须先选一个存档。 */}
      {isDatapack &&
        ((worlds ?? []).length > 0 ? (
          <label className="flex items-center gap-[8px] text-[12px] text-muted">
            <span className="shrink-0">{t("instance.targetWorld")}</span>
            <Select
              className="flex-1 !min-w-0"
              value={world ?? ""}
              onChange={(v) => setWorld(v)}
              options={(worlds ?? []).map((w) => ({ value: w.folder, label: w.name || w.folder }))}
            />
          </label>
        ) : (
          <div className="text-[12px] leading-[1.7] text-muted py-[4px]">{t("instance.datapackNoWorlds")}</div>
        ))}

      {props.browse ? (
        // 浏览模式 = 复用探索页:搜索列表 →(点进)详情安装,装完回到已安装。
        detail ? (
          <ProjectInstallDetail
            hit={detail}
            kind={props.searchKind as Exclude<ProjectKind, "modpack">}
            provider={detailProvider}
            lockedInstance={props.instance}
            onBack={() => setDetail(null)}
            onInstalled={() => {
              refetch();
              if (detail) setAdded((s) => new Set(s).add(detail.id));
            }}
          />
        ) : (
          <>
            <button
              className="self-start inline-flex items-center gap-[4px] h-[28px] px-[10px] rounded-none border-none bg-transparent text-muted text-[12px] cursor-pointer transition-colors duration-150 hover:bg-panel-3 hover:text-fg"
              onClick={() => {
                setDetail(null);
                props.onBrowse(false);
              }}
            >
              {t("instance.backToInstalled")}
            </button>
            <ContentBrowser
              kind={props.searchKind}
              mcVersion={props.instance.mc_version}
              loader={null}
              onOpenDetail={(hit, provider) => {
                setDetail(hit);
                setDetailProvider(provider);
              }}
              onAdd={(hit, provider) => install(hit.id, hit.title, provider)}
              addingIds={installing}
              addedIds={added}
              disabledReason={
                isDatapack ? () => (worldArg ? null : t("instance.selectTargetWorldFirst")) : undefined
              }
              autofocus
              onEscape={() => props.onBrowse(false)}
              placeholder={t("instance.searchModrinth", { version: props.instance.mc_version })}
            />
          </>
        )
      ) : (
        <>
          {/* 默认:「已安装」标题行,右侧聚拢动作(打开目录 + 紧凑「添加」)。 */}
          <div className="flex items-center justify-between">
            <div className={LABEL}>{t("instance.installedTitle")}</div>
            <div className="flex items-center gap-[6px]">
              <button
                className={OPEN_BTN}
                onClick={() =>
                  openInstanceSubdir(
                    activeRoot(),
                    props.instance.id,
                    props.kind === "resource_pack"
                      ? "resourcepacks"
                      : props.kind === "shader"
                        ? "shaderpacks"
                        : world
                          ? `saves/${world}/datapacks`
                          : "datapacks",
                  )
                }
              >
                {t("instance.openDir")}
              </button>
              <button
                className="shrink-0 h-[28px] px-[10px] rounded-none bg-accent text-white shadow-raised text-[12px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed"
                onClick={startBrowse}
              >
                {t("instance.add")}
              </button>
            </div>
          </div>

          {packsLoading ? (
            <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
              <Spinner size={16} /> {t("instance.scanning")}
            </div>
          ) : (packs ?? []).length > 0 ? (
            <div className="flex flex-col gap-[6px]">
              {packs!.map((p) => (
                <div
                  key={p.file_name}
                  className={clsx("flex items-center gap-[10px] py-[8px] px-[10px] rounded-none bg-panel-2", {
                    "opacity-55": !p.enabled,
                  })}
                >
                  <div className="flex-1 min-w-0">
                    <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                      {p.file_name.replace(/\.disabled$/, "")}
                    </div>
                    <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                      {[p.description, fmtSize(p.size)].filter(Boolean).join(" · ")}
                    </div>
                  </div>
                  <div className="flex items-center shrink-0">
                    <Toggle checked={p.enabled} onChange={(v) => toggle(p, v)} title={t("instance.enable")} />
                  </div>
                  <button className={DEL_BTN} onClick={() => setConfirmDel(p)}>
                    {t("instance.delete")}
                  </button>
                </div>
              ))}
            </div>
          ) : packsError ? (
            <ErrorState compact message={t("instance.loadFailedShort")} onRetry={() => void refetch()} />
          ) : (
            <div className="flex flex-col items-center justify-center gap-[12px] py-[40px] text-center">
              <div className="text-muted text-[13px]">{props.emptyHint}</div>
              <button className={ACCENT_BTN} onClick={startBrowse}>
                {t("instance.add")}
              </button>
            </div>
          )}
        </>
      )}

      <Dialog
        open={confirmDel !== null}
        onClose={() => setConfirmDel(null)}
        label={t("instance.deleteResourcePack")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <div className="text-[15px] font-semibold text-fg break-words">
            {t("instance.deleteFileConfirm", { file: confirmDel?.file_name.replace(/\.disabled$/, "") ?? "" })}
          </div>
          <div className="text-[13px] text-muted leading-[1.6]">{t("instance.deleteFileBody")}</div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDel(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const p = confirmDel;
                setConfirmDel(null);
                if (p) void remove(p);
              }}
            >
              {t("instance.delete")}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}

/**
 * WorldsPanel —— 存档世界列表 + 备份(导出 zip)/ 重命名(改显示名)/ 删除(走回收站)。
 */
function WorldsPanel(props: { instance: InstanceSummary; tick?: number }) {
  const { data: worlds, loading: worldsLoading, error: worldsError, refetch } = useAsync<WorldInfo[]>(
    () => api.instanceWorlds(activeRoot(), props.instance.id),
    [props.instance.id, props.tick ?? 0],
  );

  // 行内重命名:正在编辑的世界 folder + 草稿名。
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  // 删除存档前确认(存档含游玩进度,删除是破坏性的)。
  const [confirmDel, setConfirmDel] = useState<WorldInfo | null>(null);
  // commitRename 的防重入要读最新 editing(否则 onBlur 的旧闭包会误判为仍在编辑本行)。
  const editingRef = useRef(editing);
  editingRef.current = editing;

  async function importZip() {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: t("instance.worldsZipFilter"), extensions: ["zip"] }],
      title: t("instance.pickWorldZip"),
    });
    if (typeof picked !== "string") return;
    setImporting(true);
    try {
      const folder = await api.importWorldZip(activeRoot(), props.instance.id, picked);
      toast({ type: "success", message: t("instance.importedWorld", { folder }) });
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.importFailed", { err: String(e) }) });
    } finally {
      setImporting(false);
    }
  }

  async function remove(w: WorldInfo) {
    try {
      await api.deleteWorld(activeRoot(), props.instance.id, w.folder);
      toast({ type: "success", message: t("instance.deletedWorld", { name: w.name }) });
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(e) }) });
    }
  }

  async function backup(w: WorldInfo) {
    // 另存为:用户自定文件名/位置;同名文件由系统对话框确认覆盖,不会静默盖掉上次备份。
    const dest = await saveDialog({
      title: t("instance.backupWorldAs"),
      defaultPath: `${(w.name || w.folder).replace(/[\\/:*?"<>|]/g, "_")}-backup.zip`,
      filters: [{ name: t("instance.zipBackup"), extensions: ["zip"] }],
    });
    if (!dest) return; // 取消
    setBusy(w.folder);
    try {
      const zip = await api.backupWorld(activeRoot(), props.instance.id, w.folder, dest);
      toast({ type: "success", message: t("instance.backedUpTo", { zip }) });
    } catch (e) {
      toast({ type: "error", message: t("instance.backupFailed", { err: String(e) }) });
    } finally {
      setBusy(null);
    }
  }

  function startRename(w: WorldInfo) {
    setDraft(w.name);
    setEditing(w.folder);
  }

  async function commitRename(w: WorldInfo) {
    // 防重入:Enter 提交成功后会 setEditing(null),输入框卸载又触发 onBlur 二次调用;
    // Escape 也先 setEditing(null) 再触发 onBlur。两种情况此时 editing 已不是本行,
    // 直接返回 —— 避免重复重命名/重复 toast,以及「Escape 反而保存」。
    if (editingRef.current !== w.folder) return;
    const name = draft.trim();
    if (!name || name === w.name) {
      setEditing(null);
      return;
    }
    try {
      await api.renameWorld(activeRoot(), props.instance.id, w.folder, name);
      toast({ type: "success", message: t("instance.renamedTo", { name }) });
      setEditing(null);
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.renameFailed", { err: String(e) }) });
    }
  }

  const MODE_LABEL: Record<string, string> = {
    survival: t("instance.modeSurvival"),
    creative: t("instance.modeCreative"),
    adventure: t("instance.modeAdventure"),
    spectator: t("instance.modeSpectator"),
    unknown: t("instance.modeUnknown"),
  };

  return (
    <div className="flex flex-col gap-[8px]">
      <div className="flex items-center justify-between">
        <div className={LABEL}>{t("instance.worldsListTitle")}</div>
        <div className="flex items-center gap-[4px]">
          <button
            className={OPEN_BTN}
            onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "saves")}
          >
            {t("instance.openDir")}
          </button>
          <button
            className="text-[12px] text-accent px-[8px] py-[3px] rounded-none cursor-pointer hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
            disabled={importing}
            onClick={importZip}
          >
            {importing ? t("instance.importingWorld") : t("instance.importWorld")}
          </button>
        </div>
      </div>

      {worldsLoading ? (
        <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
          <Spinner size={16} /> {t("instance.scanningWorlds")}
        </div>
      ) : (worlds ?? []).length > 0 ? (
        <div className="flex flex-col gap-[6px]">
          {worlds!.map((w) => (
            <div key={w.folder} className="bg-panel-2 shadow-sunken flex items-center gap-[10px] py-[8px] px-[10px] rounded-none">
              <div className="flex-1 min-w-0">
                {editing === w.folder ? (
                  <input
                    className={`${FIELD} h-[26px] w-full text-[12px]`}
                    ref={(el) => {
                      if (el) queueMicrotask(() => el.focus());
                    }}
                    name="worldName"
                    autoComplete="off"
                    spellCheck={false}
                    value={draft}
                    onChange={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename(w);
                      else if (e.key === "Escape") setEditing(null);
                    }}
                    onBlur={() => commitRename(w)}
                  />
                ) : (
                  <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">{w.name}</div>
                )}
                <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                  {[
                    MODE_LABEL[w.game_mode] ?? w.game_mode,
                    fmtSize(w.size_bytes),
                    w.seed != null ? t("instance.seed", { seed: w.seed }) : null,
                    w.folder,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </div>
              </div>
              <button
                className="shrink-0 text-[12px] text-muted px-[8px] py-[4px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
                disabled={busy === w.folder}
                onClick={() => backup(w)}
              >
                {busy === w.folder ? t("instance.backingUp") : t("instance.backup")}
              </button>
              <button
                className="shrink-0 text-[12px] text-muted px-[8px] py-[4px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2"
                onClick={() => startRename(w)}
              >
                {t("instance.rename")}
              </button>
              <button className={DEL_BTN} onClick={() => setConfirmDel(w)}>
                {t("instance.delete")}
              </button>
            </div>
          ))}
        </div>
      ) : worldsError ? (
        <ErrorState compact message={t("instance.worldsLoadError")} onRetry={() => void refetch()} />
      ) : (
        <div className="text-muted text-[13px] py-[12px]">{t("instance.noWorlds")}</div>
      )}

      <Dialog
        open={confirmDel !== null}
        onClose={() => setConfirmDel(null)}
        label={t("instance.deleteWorld")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <div className="text-[15px] font-semibold text-fg">
            {t("instance.deleteWorldConfirm", { name: confirmDel?.name ?? "" })}
          </div>
          <div className="text-[13px] text-muted leading-[1.6]">{t("instance.deleteWorldBody")}</div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDel(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const w = confirmDel;
                setConfirmDel(null);
                if (w) void remove(w);
              }}
            >
              {t("instance.delete")}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}

/** 加载器选项(与后端 parse_loader_kind 对齐)。 */
const LOADER_OPTS = [
  { label: "Fabric", value: "fabric" },
  { label: "Quilt", value: "quilt" },
  { label: "Forge", value: "forge" },
  { label: "NeoForge", value: "neoforge" },
];

/**
 * AddLoaderPanel —— 原版实例「加装核心」:选加载器(+ Forge/NeoForge 版本)→ install_loader。
 * 装完后端可能换实例 id(实例目录名恰为原版号的退化情形),回调把新 id 传出去重定向。
 */
function AddLoaderPanel(props: { instance: InstanceSummary; onAdded: (newId: string) => void }) {
  const [loader, setLoader] = useState("fabric");
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState("");
  const needsVersion = loader === "forge" || loader === "neoforge";
  // 进度回调装一次即可,却要读最新 busy —— 用 ref 避免旧闭包永远看到 busy=false。
  const busyRef = useRef(busy);
  busyRef.current = busy;

  useEffect(
    () =>
      onInstallProgress((p) => {
        if (!busyRef.current) return;
        setProgress(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage);
      }),
    [],
  );

  async function add() {
    if (busy) return;
    if (needsVersion && !version.trim()) {
      toast({ type: "error", message: t("instance.fillForgeVersion") });
      return;
    }
    setBusy(true);
    setProgress(t("instance.preparing"));
    try {
      const newId = await api.installLoader(
        activeRoot(),
        props.instance.id,
        props.instance.mc_version,
        loader,
        needsVersion ? version.trim() : null,
      );
      toast({ type: "success", message: t("instance.loaderAdded") });
      props.onAdded(newId);
    } catch (e) {
      toast({ type: "error", message: t("instance.addLoaderFailed", { err: String(e) }) });
    } finally {
      setBusy(false);
      setProgress("");
    }
  }

  return (
    <div className="flex flex-col gap-[10px] py-[4px]">
      <div className="text-[13px] text-muted leading-[1.6]">{t("instance.addLoaderIntro")}</div>
      <div className="flex items-center gap-[8px]">
        <Select value={loader} onChange={setLoader} options={LOADER_OPTS} />
        {needsVersion && (
          <input
            className={`${FIELD} flex-1`}
            placeholder={loader === "forge" ? t("instance.forgeBuildPlaceholder") : t("instance.neoforgeVersionPlaceholder")}
            value={version}
            onChange={(e) => setVersion(e.currentTarget.value)}
          />
        )}
        <button
          className="shrink-0 h-[34px] px-[14px] rounded-none bg-accent text-white shadow-raised text-[13px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed disabled:opacity-50 disabled:cursor-default"
          disabled={busy}
          onClick={add}
        >
          {busy ? t("instance.installing") : t("instance.addLoader")}
        </button>
      </div>
      {busy && progress && (
        <div className="flex items-center gap-[8px] text-accent text-[12px]">
          <Spinner size={14} /> {progress}
        </div>
      )}
    </div>
  );
}

export function InstanceManageDialog(props: {
  open: boolean;
  instance: InstanceSummary | null;
  /** 关闭(仅非内嵌的 Dialog 模式使用;内嵌详情页无「完成」按钮)。 */
  onClose?: () => void;
  onChanged?: () => void;
  /** 复制完成回调,带新实例 id;调用方据此重拉列表并选中新实例。 */
  onCopied?: (newId: string) => void;
  /** 内嵌模式:不套 Dialog,直接铺在父容器里(实例详情页的设置标签用),
   *  隐藏实例名头部与「完成」按钮,父组件只在需要时挂载本组件即等于「打开」。 */
  embedded?: boolean;
  /** 受控 tab:启动页把实例管理页签提升到实例头部时使用。 */
  tab?: InstanceManageTab;
  onTabChange?: (tab: InstanceManageTab) => void;
  /** 隐藏本组件自带 tab 条,由外层渲染同级导航。 */
  hideTabs?: boolean;
  /** 进入/退出「添加」浏览模式(复用探索页占满内容区)时通知外层,详情页据此隐藏头部。 */
  onBrowsingChange?: (browsing: boolean) => void;
}) {
  useLang();
  const socialOn = useAppStore((s) => s.socialEnabled);
  const kobeSignedIn = useAppStore((s) => s.kobeUser !== null);

  const [internalTab, setInternalTab] = useState<InstanceManageTab>("settings");
  const [cfg, setCfg] = useState<InstanceConfig | null>(null);
  const [copying, setCopying] = useState(false);
  // 内存设置辅助:本机物理内存总量(MiB,一次即可)与按本实例 mod 数推荐的最大堆(MiB)。
  const [sysTotalMb, setSysTotalMb] = useState<number | null>(null);
  const [suggestedMb, setSuggestedMb] = useState<number | null>(null);
  // 「浏览/添加」模式:任一内容标签点「+ 添加」即进入,占满内容区(复用探索页)。
  const [browsing, setBrowsing] = useState(false);
  // keep-alive:记录已访问过的标签(惰性挂载——首访才挂,之后常驻、切走只以 display:none 隐藏)。
  const [visited, setVisited] = useState<Set<InstanceManageTab>>(new Set());

  const tab = props.tab ?? internalTab;
  const setTab = (next: InstanceManageTab) => {
    setInternalTab(next);
    props.onTabChange?.(next);
  };
  // 整合包来源(modrinth / curseforge)→ 多一个「概览」标签并置于首位。
  const modpackSource = (() => {
    const s = cfg?.source;
    return s && (s.provider === "modrinth" || s.provider === "curseforge") ? s : null;
  })();
  // 领域实例:多一个「领域」标签置于首位。仅在「社交开启 + 已登录 kobeMC」时把领域当作领域。
  const isRealm = !!props.instance?.realm && socialOn && kobeSignedIn;
  const visibleTabs = (): { key: InstanceManageTab; label: string }[] => {
    if (isRealm) {
      return [
        { key: "realm", label: t("instance.tabRealm") },
        ...(modpackSource ? [{ key: "overview" as const, label: t("instance.tabOverview") }] : []),
        { key: "mods", label: t("instance.tabMods") },
        { key: "resource_pack", label: t("instance.tabResourcePack") },
        { key: "shader", label: t("instance.tabShader") },
        { key: "datapack", label: t("instance.tabDatapack") },
        { key: "worlds", label: t("instance.tabWorlds") },
        { key: "servers", label: t("instance.tabServers") },
        { key: "settings", label: t("instance.tabSettings") },
        { key: "screenshots", label: t("instance.tabScreenshots") },
      ];
    }
    return modpackSource ? [{ key: "overview", label: t("instance.tabOverview") }, ...TABS()] : TABS();
  };

  // 是否「活动」(应加载数据 / 接受拖放):弹窗模式看 open,内嵌模式只要挂载即活动。
  const active = props.embedded || props.open;

  // 通知外层浏览态变化(详情页隐藏头部 + 本组件隐藏 tab 条)。
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => props.onBrowsingChange?.(browsing), [browsing]);

  // 首访即登记该标签(之后常驻);functional setState 免把 visited 放进 deps。
  useEffect(() => {
    if (active) setVisited((s) => (s.has(tab) ? s : new Set(s).add(tab)));
  }, [tab, active]);

  async function copyInstance() {
    const inst = props.instance;
    if (!inst) return;
    // 运行中复制会把正在写入的存档/文件拷成半截 —— 先要求停止。
    if (isRunning(inst.id)) {
      toast({ type: "error", message: t("instance.stopBeforeCopy") });
      return;
    }
    setCopying(true);
    try {
      const newId = await api.copyInstance(activeRoot(), inst.id, t("instance.copyName", { name: inst.name || inst.id }));
      toast({ type: "success", message: t("instance.copiedInstance") });
      props.onCopied?.(newId);
    } catch (e) {
      toast({ type: "error", message: t("instance.copyFailed", { err: String(e) }) });
    } finally {
      setCopying(false);
    }
  }

  // 系统内存只取一次;推荐值按本实例 mod 数计算,随实例变化。sysTotalMb 走 ref 读——否则本
  // effect 会订阅它自己 set 的 sysTotalMb,首次解析(null→值)重跑整个 effect、
  // 重复拉 getInstanceConfig/suggestInstanceMemory 并闪一下配置 spinner。
  const sysTotalRef = useRef<number | null>(null);

  // 打开/切换实例时拉配置 + 复位到设置页;关闭时清空。
  useEffect(() => {
    const inst = props.instance;
    setUpdates(null); // 切换实例/开关时清掉上一个实例的更新检查结果。
    if (active && inst) {
      setCfg(null);
      setSuggestedMb(null);
      api
        .getInstanceConfig(activeRoot(), inst.id)
        .then((c) => {
          setCfg(c);
          // 默认标签(仅非受控时):领域实例落「领域」,整合包来源落「概览」,其余落「设置」。
          if (props.tab === undefined)
            setInternalTab(
              isRealm ? "realm"
              : c.source?.provider === "modrinth" || c.source?.provider === "curseforge" ? "overview"
              : "settings",
            );
        })
        .catch((e) => toast({ type: "error", message: t("instance.readConfigFailed", { err: String(e) }) }));
      if (sysTotalRef.current === null)
        api
          .systemMemory()
          .then((m) => {
            sysTotalRef.current = m.total_mb;
            setSysTotalMb(m.total_mb);
          })
          .catch(() => {});
      api.suggestInstanceMemory(activeRoot(), inst.id).then(setSuggestedMb).catch(() => {});
    } else if (!active) {
      setCfg(null);
      setTab("settings");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.instance, active]);

  // Mods:仅在 Mods 标签 + 弹窗打开时拉取(gate:未满足条件不真正打后端)。
  const modsGated = active && !!props.instance && visited.has("mods");
  const { data: mods, loading: modsLoading, error: modsError, refetch: refetchMods } = useAsync<ModInfo[] | undefined>(
    () => (modsGated && props.instance ? api.instanceMods(activeRoot(), props.instance.id) : Promise.resolve(undefined)),
    [modsGated, props.instance?.id],
  );

  // ---- 从 Modrinth 搜索并安装 ----
  // vanilla 实例没有加载器,搜 mod 无意义,这里把 loader 归一为 null(不限)。
  const searchLoader = (() => {
    const l = props.instance?.loader;
    return l && l !== "vanilla" ? l : null;
  })();
  const [modDetail, setModDetail] = useState<ModpackHit | null>(null);
  const [modDetailProvider, setModDetailProvider] = useState<ContentProvider>("modrinth");
  // 后台并行安装:正在安装的 project_id 集合(不阻塞其它行)。
  const [installing, setInstalling] = useState<Set<string>>(new Set());
  // 本次浏览已添加的 mod project_id:行按钮即时变「已添加」。
  const [addedMods, setAddedMods] = useState<Set<string>>(new Set());
  // 删除 mod 前确认(删除是破坏性的,与存档/资源包删除一致)。
  const [confirmDelMod, setConfirmDelMod] = useState<ModInfo | null>(null);
  const startBrowse = () => {
    setAddedMods(new Set<string>());
    setBrowsing(true);
  };
  // 切换标签(含外层受控切换)即退出浏览/添加模式并清掉详情;defer:首挂不跑(初值已是复位态)。
  const tabResetFirst = useRef(true);
  useEffect(() => {
    if (tabResetFirst.current) {
      tabResetFirst.current = false;
      return;
    }
    setBrowsing(false);
    setModDetail(null);
  }, [tab]);

  // 行内「下载」:直接装最新兼容版(解析依赖),不进详情;后台并行,不阻塞其它行。
  async function installHit(projectId: string, title: string, provider: ContentProvider = "modrinth") {
    const inst = props.instance;
    if (!inst || installing.has(projectId)) return;
    setInstalling((s) => new Set(s).add(projectId));
    try {
      const report = await api.installMod(activeRoot(), inst.id, projectId, inst.mc_version, searchLoader ?? "", provider);
      if ((report.blocked?.length ?? 0) > 0) {
        toast({ type: "warn", message: t("instance.blockedManual", { count: report.blocked!.length }) });
      } else {
        if (report.installed.length === 0 && report.unresolved.length === 0) {
          toast({ type: "info", message: t("instance.modExists", { title }) });
        } else {
          const parts = [t("instance.modInstalledCount", { n: report.installed.length })];
          if (report.unresolved.length > 0) parts.push(t("instance.modUnresolvedCount", { n: report.unresolved.length }));
          toast({
            type: report.unresolved.length > 0 ? "warn" : "success",
            message: t("instance.modInstallResult", { title, parts: parts.join(",") }),
          });
        }
        setAddedMods((s) => new Set(s).add(projectId));
        refetchMods();
      }
    } catch (e) {
      toast({ type: "error", message: t("instance.installFailed", { err: String(e) }) });
    } finally {
      setInstalling((s) => {
        const n = new Set(s);
        n.delete(projectId);
        return n;
      });
    }
  }

  // ---- Mod 更新检查 ----
  const [updates, setUpdates] = useState<ModUpdate[] | null>(null);
  const [checking, setChecking] = useState(false);
  // 后台并行更新:正在更新的文件集合(不阻塞其它行/全部更新串行)。
  const [updating, setUpdating] = useState<Set<string>>(new Set());

  async function checkUpdates() {
    const inst = props.instance;
    if (!inst) return;
    setChecking(true);
    try {
      const list = await api.checkModUpdates(activeRoot(), inst.id, inst.mc_version, searchLoader ?? "");
      setUpdates(list);
      toast({
        type: list.length > 0 ? "info" : "success",
        message: list.length > 0 ? t("instance.foundUpdates", { n: list.length }) : t("instance.allModsUpToDate"),
      });
    } catch (e) {
      toast({ type: "error", message: t("instance.checkUpdatesFailed", { err: String(e) }) });
    } finally {
      setChecking(false);
    }
  }

  async function applyUpdate(u: ModUpdate) {
    const inst = props.instance;
    if (!inst || updating.has(u.file_name)) return;
    setUpdating((s) => new Set(s).add(u.file_name));
    try {
      await api.applyModUpdate(activeRoot(), inst.id, u);
      toast({ type: "success", message: t("instance.modUpdated", { name: u.name, version: u.new_version }) });
      setUpdates((prev) => (prev ?? []).filter((x) => x.file_name !== u.file_name));
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: t("instance.updateFailed", { err: String(e) }) });
    } finally {
      setUpdating((s) => {
        const n = new Set(s);
        n.delete(u.file_name);
        return n;
      });
    }
  }

  async function applyAllUpdates() {
    // 后台并行更新,不阻塞单项;每个失败只提示该项,不中断其余。
    await Promise.all((updates ?? []).map((u) => applyUpdate(u)));
  }

  // ---- 拖拽导入 ----
  // Tauri 启用了原生文件拖放,HTML5 ondrop 不触发,改用 webview 的 onDragDropEvent。
  const [dragOver, setDragOver] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [importTick, setImportTick] = useState(0);
  const [worldTick, setWorldTick] = useState(0);

  // 拖放事件在监听装好之后才触发,需读「当时」的 tab/instance/active —— 走 ref 取实时值,
  // 让监听只随 active 重订阅(镜像 Solid effect 只追踪 active())。
  const tabRef = useRef(tab);
  tabRef.current = tab;
  const activeRef = useRef(active);
  activeRef.current = active;
  const instanceRef = useRef(props.instance);
  instanceRef.current = props.instance;

  /** 当前标签接受拖拽吗(设置标签不接受)。 */
  function dropAcceptedFor(cur: InstanceManageTab): boolean {
    return cur === "mods" || isPackTab(cur) || cur === "worlds";
  }

  /** mods/资源包/光影/数据包的导入目标类型(存档走单独的 zip 导入命令,这里返回 null)。 */
  function resourceTargetFor(cur: InstanceManageTab): string | null {
    if (cur === "mods") return "mod";
    if (isPackTab(cur)) return cur === "resource_pack" ? "resourcepack" : cur;
    return null;
  }

  async function handleDrop(paths: string[]) {
    const inst = instanceRef.current;
    const cur = tabRef.current;
    if (!inst || !dropAcceptedFor(cur)) {
      toast({ type: "info", message: t("instance.dropHint") });
      return;
    }
    setDropping(true);
    try {
      // 并行导入(串行会让拖入多个大文件逐个卡住);用 allSettled 汇总成败。
      const results = await Promise.allSettled(
        paths.map((path) =>
          cur === "worlds"
            ? api.importWorldZip(activeRoot(), inst.id, path)
            : api.importLocalResource(activeRoot(), inst.id, resourceTargetFor(cur)!, path, null),
        ),
      );
      const ok = results.filter((r) => r.status === "fulfilled").length;
      const failed = results.length - ok;
      if (ok > 0) {
        if (cur === "mods") refetchMods();
        else if (cur === "worlds") setWorldTick((x) => x + 1);
        else setImportTick((x) => x + 1);
      }
      // 单条汇总,而不是每个失败弹一条 + 末尾静默。
      if (failed === 0) toast({ type: "success", message: t("instance.importedFiles", { n: ok }) });
      else if (ok === 0) toast({ type: "error", message: t("instance.importFilesFailed", { n: failed }) });
      else toast({ type: "warn", message: t("instance.importFilesPartial", { ok, failed }) });
    } finally {
      setDropping(false);
    }
  }

  useEffect(() => {
    if (!active) return;
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      if (!activeRef.current) return;
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") setDragOver(true);
      else if (p.type === "leave") setDragOver(false);
      else if (p.type === "drop") {
        setDragOver(false);
        void handleDrop(p.paths);
      }
    });
    return () => void unlisten.then((f) => f());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  function patch(p: Partial<InstanceConfig>) {
    const cur = cfg;
    const inst = props.instance;
    if (!cur || !inst) return;
    const next = { ...cur, ...p };
    setCfg(next);
    void api
      .setInstanceConfig(activeRoot(), inst.id, next)
      .then(() => props.onChanged?.())
      .catch((e) => toast({ type: "error", message: t("instance.saveFailed", { err: String(e) }) }));
  }

  // MiB → 友好的 GB 文本(整数不带小数,否则保留一位)。
  const memGb = (mb: number): string => {
    const v = mb / 1024;
    return Number.isInteger(v) ? `${v}` : v.toFixed(1);
  };

  // 把内存滑块设为后端按系统内存 + mod 数推荐的值。
  function applyRecommendedMemory() {
    if (suggestedMb == null) return;
    patch({ memory_mb: suggestedMb });
  }

  async function pickIcon() {
    const inst = props.instance;
    if (!inst) return;
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: t("instance.imageFilter"), extensions: ["png", "jpg", "jpeg", "gif", "bmp", "webp"] }],
    });
    if (typeof picked !== "string") return; // 取消 / 多选(不会发生)
    try {
      await api.setInstanceIcon(activeRoot(), inst.id, picked);
      toast({ type: "success", message: t("instance.iconUpdated") });
      props.onChanged?.(); // 触发列表重拉,新图标随 list_instances 探测回来
    } catch (e) {
      toast({ type: "error", message: t("instance.setIconFailed", { err: String(e) }) });
    }
  }

  async function toggleMod(m: ModInfo, enabled: boolean) {
    const inst = props.instance;
    if (!inst) return;
    try {
      await api.setModEnabled(activeRoot(), inst.id, m.file_name, enabled);
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: t("instance.opFailed", { err: String(e) }) });
    }
  }

  async function removeMod(m: ModInfo) {
    const inst = props.instance;
    if (!inst) return;
    try {
      await api.deleteMod(activeRoot(), inst.id, m.file_name);
      toast({ type: "success", message: t("instance.deletedMod", { name: m.name }) });
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: t("instance.deleteModFailed", { err: String(e) }) });
    }
  }

  const dropAccepted = dropAcceptedFor(tab);

  const body = (
    <div
      className={clsx("relative flex flex-col transition-shadow duration-150", {
        "max-h-[calc(100vh-100px)]": !props.embedded,
        "h-full": props.embedded,
        "ring-2 ring-inset ring-accent": dragOver,
      })}
    >
      {dragOver && dropAccepted && (
        <div className="absolute inset-0 z-10 grid place-items-center bg-window/85 pointer-events-none">
          <div className="text-[14px] text-accent font-semibold">{t("instance.dropToImport")}</div>
        </div>
      )}
      {dropping && (
        <div className="absolute inset-0 z-10 grid place-items-center bg-window/85">
          <div className="flex items-center gap-[10px] text-[14px] text-fg font-semibold">
            <Spinner size={18} /> {t("instance.importingOverlay")}
          </div>
        </div>
      )}
      {!props.embedded && (
        <Heading size="sub" className="px-[20px] pt-[18px]">
          {props.instance?.name || props.instance?.id}
        </Heading>
      )}

      {!props.hideTabs && !browsing && (
        <div className="shrink-0 flex gap-[4px] px-[16px] border-b border-titlebar mt-[10px] overflow-x-auto">
          {visibleTabs().map((item) => (
            <button
              key={item.key}
              className={`${TAB} whitespace-nowrap ${tab === item.key ? TAB_ACTIVE : ""}`}
              onClick={() => setTab(item.key)}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}

      <div className="flex-1 min-h-0 p-[20px] flex flex-col gap-[14px] overflow-y-auto">
        {/* ---- 领域(同步 / 成员;领域实例的主标签)---- */}
        {visited.has("realm") && isRealm && props.instance && (
          <div className={clsx({ hidden: tab !== "realm" })}>
            <RealmPanel instance={props.instance} onChanged={() => props.onChanged?.()} />
          </div>
        )}

        {/* ---- 概览(整合包来源)---- */}
        {visited.has("overview") && modpackSource && (
          <div className={clsx({ hidden: tab !== "overview" })}>
            <ModpackOverview projectId={modpackSource.project_id} provider={modpackSource.provider} />
          </div>
        )}

        {/* ---- 设置 ---- */}
        {visited.has("settings") && (
          <div className={clsx("flex flex-col gap-[14px]", { hidden: tab !== "settings" })}>
            {cfg ? (
              <>
                <div className="flex items-center gap-[12px]">
                  <div className="w-[56px] h-[56px] rounded-none overflow-hidden bg-panel-2 shrink-0 select-none">
                    <InstanceIcon name={props.instance?.name || props.instance?.id} icon={props.instance?.icon ?? undefined} />
                  </div>
                  <div className="flex flex-col gap-[5px]">
                    <span className={LABEL}>{t("instance.instanceIcon")}</span>
                    <button
                      className="h-[30px] px-[12px] shadow-raised rounded-none bg-panel-3 text-fg text-[12px] cursor-pointer transition-[box-shadow,filter] duration-[var(--dur)] ease-app hover:brightness-110 active:shadow-pressed w-fit"
                      onClick={pickIcon}
                    >
                      {t("instance.changeIcon")}
                    </button>
                  </div>
                </div>

                <label className="flex flex-col gap-[5px]">
                  <span className={LABEL}>{t("instance.name")}</span>
                  {/* 非受控 + onBlur 持久化:自由输入,失焦才写盘(等价 Solid 的 onChange)。 */}
                  <input
                    key={`name-${props.instance?.id ?? ""}`}
                    className={FIELD}
                    defaultValue={cfg.name ?? ""}
                    onBlur={(e) => patch({ name: e.currentTarget.value || null })}
                  />
                </label>

                <div className="flex flex-col gap-[5px]">
                  <div className="flex items-center justify-between gap-[8px]">
                    <span className={LABEL}>{t("instance.maxMemory", { mb: cfg.memory_mb ?? 0 })}</span>
                    <div className="flex items-center gap-[8px]">
                      {sysTotalMb !== null && (
                        <span className="text-muted text-[11px]">{t("instance.systemMemory", { gb: memGb(sysTotalMb) })}</span>
                      )}
                      {suggestedMb !== null && (
                        <button
                          type="button"
                          className="h-[22px] px-[8px] rounded-none bg-panel-3 text-fg text-[11px] cursor-pointer shadow-raised hover:brightness-110 active:shadow-pressed transition-[box-shadow,filter] duration-[var(--dur)] ease-app"
                          title={t("instance.recommendMemoryHint")}
                          onClick={applyRecommendedMemory}
                        >
                          {t("instance.recommendMemory", { gb: memGb(suggestedMb) })}
                        </button>
                      )}
                    </div>
                  </div>
                  {/* 拖动时 onChange 只更新本地(实时刻度);松手(mouseup/keyup)才写盘,避免逐帧持久化。 */}
                  <input
                    className="kb-range"
                    type="range"
                    min={512}
                    max={16384}
                    step={256}
                    value={cfg.memory_mb}
                    onChange={(e) => {
                      const v = +e.currentTarget.value;
                      setCfg((prev) => (prev ? { ...prev, memory_mb: v } : prev));
                    }}
                    onMouseUp={(e) => patch({ memory_mb: +e.currentTarget.value })}
                    onKeyUp={(e) => patch({ memory_mb: +e.currentTarget.value })}
                  />
                </div>

                <label className="flex flex-col gap-[5px]">
                  <span className={LABEL}>{t("instance.javaPath")}</span>
                  <input
                    key={`java-${props.instance?.id ?? ""}`}
                    className={FIELD}
                    placeholder={t("instance.javaPathPlaceholder")}
                    defaultValue={cfg.java_path ?? ""}
                    onBlur={(e) => patch({ java_path: e.currentTarget.value || null })}
                  />
                </label>

                <label className="flex flex-col gap-[5px]">
                  <span className={LABEL}>{t("instance.extraJvmArgs")}</span>
                  <input
                    key={`jvm-${props.instance?.id ?? ""}`}
                    className={FIELD}
                    defaultValue={(cfg.jvm_args ?? []).join(" ")}
                    onBlur={(e) => patch({ jvm_args: e.currentTarget.value.split(/\s+/).filter(Boolean) })}
                  />
                </label>

                <div className="flex gap-[12px]">
                  <label className="flex-1 flex flex-col gap-[5px]">
                    <span className={LABEL}>{t("instance.windowWidth")}</span>
                    <input
                      key={`w-${props.instance?.id ?? ""}`}
                      className={FIELD}
                      type="number"
                      min={1}
                      max={7680}
                      placeholder={t("instance.defaultPlaceholder")}
                      defaultValue={cfg.width ?? ""}
                      onBlur={(e) => {
                        const n = Math.floor(+e.currentTarget.value);
                        patch({ width: Number.isFinite(n) && n > 0 ? n : null });
                      }}
                    />
                  </label>
                  <label className="flex-1 flex flex-col gap-[5px]">
                    <span className={LABEL}>{t("instance.windowHeight")}</span>
                    <input
                      key={`h-${props.instance?.id ?? ""}`}
                      className={FIELD}
                      type="number"
                      min={1}
                      max={4320}
                      placeholder={t("instance.defaultPlaceholder")}
                      defaultValue={cfg.height ?? ""}
                      onBlur={(e) => {
                        const n = Math.floor(+e.currentTarget.value);
                        patch({ height: Number.isFinite(n) && n > 0 ? n : null });
                      }}
                    />
                  </label>
                </div>

                <div className="flex items-center justify-between text-fg text-[13px]">
                  <span>{t("instance.fullscreenLaunch")}</span>
                  <Toggle
                    checked={cfg.fullscreen ?? false}
                    onChange={(v) => patch({ fullscreen: v })}
                    title={t("instance.fullscreenLaunch")}
                  />
                </div>

                <div className="pt-[4px]">
                  <button
                    className="h-[30px] px-[12px] shadow-raised rounded-none bg-panel-3 text-fg text-[12px] cursor-pointer transition-[box-shadow,filter] duration-[var(--dur)] ease-app hover:brightness-110 active:shadow-pressed"
                    onClick={() => props.instance && openInstanceDir(activeRoot(), props.instance.id)}
                  >
                    {t("instance.openGameDir")}
                  </button>
                </div>
              </>
            ) : (
              <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
                <Spinner size={16} /> {t("instance.readingConfig")}
              </div>
            )}
          </div>
        )}

        {/* ---- Mods ---- */}
        {visited.has("mods") && (
          <div className={clsx("flex flex-col gap-[8px]", { hidden: tab !== "mods" })}>
            {searchLoader === null ? (
              <AddLoaderPanel
                instance={props.instance!}
                onAdded={(newId) => {
                  props.onChanged?.();
                  if (newId !== props.instance!.id) openInstance(newId);
                }}
              />
            ) : tab === "mods" && browsing ? (
              // 浏览模式 = 复用探索页:搜索列表 →(点进)详情安装,装完回到已安装。
              modDetail ? (
                <ProjectInstallDetail
                  hit={modDetail}
                  kind="mod"
                  provider={modDetailProvider}
                  lockedInstance={props.instance!}
                  onBack={() => setModDetail(null)}
                  onInstalled={() => {
                    refetchMods();
                    if (modDetail) setAddedMods((s) => new Set(s).add(modDetail.id));
                  }}
                />
              ) : (
                <>
                  <button
                    className="self-start inline-flex items-center gap-[4px] h-[28px] px-[10px] rounded-none border-none bg-transparent text-muted text-[12px] cursor-pointer transition-colors duration-150 hover:bg-panel-3 hover:text-fg"
                    onClick={() => setBrowsing(false)}
                  >
                    {t("instance.backToInstalled")}
                  </button>
                  <ContentBrowser
                    kind="mod"
                    mcVersion={props.instance?.mc_version ?? ""}
                    loader={searchLoader}
                    onOpenDetail={(hit, provider) => {
                      setModDetail(hit);
                      setModDetailProvider(provider);
                    }}
                    onAdd={(hit, provider) => installHit(hit.id, hit.title, provider)}
                    addingIds={installing}
                    addedIds={addedMods}
                    autofocus
                    onEscape={() => setBrowsing(false)}
                    placeholder={t("instance.searchModrinthMod", {
                      version: props.instance?.mc_version ?? "",
                      loader: searchLoader ?? t("instance.noLoader"),
                    })}
                  />
                </>
              )
            ) : (
              <>
                {/* 默认:「已安装」标题行,右侧聚拢动作(打开目录 + 检查更新 + 紧凑「添加」)。 */}
                <div className="flex items-center justify-between">
                  <div className={LABEL}>{t("instance.installedTitle")}</div>
                  <div className="flex items-center gap-[6px]">
                    <button
                      className={OPEN_BTN}
                      onClick={() => openInstanceSubdir(activeRoot(), props.instance!.id, "mods")}
                    >
                      {t("instance.openDir")}
                    </button>
                    <button
                      className="text-[12px] text-accent px-[8px] py-[3px] rounded-none cursor-pointer hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
                      disabled={checking || searchLoader === null}
                      onClick={checkUpdates}
                    >
                      {checking ? t("instance.checking") : t("instance.checkUpdates")}
                    </button>
                    <button
                      className="shrink-0 h-[28px] px-[10px] rounded-none bg-accent text-white shadow-raised text-[12px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed"
                      onClick={startBrowse}
                    >
                      {t("instance.add")}
                    </button>
                  </div>
                </div>

                {/* 可更新清单(检查后才出现) */}
                {(updates ?? []).length > 0 && (
                  <div className="flex flex-col gap-[6px] rounded-none bg-panel-2 p-[8px]">
                    <div className="flex items-center justify-between">
                      <span className="text-[12px] text-fg font-semibold">
                        {t("instance.updatesAvailable", { n: updates!.length })}
                      </span>
                      <button className={INSTALL_BTN} disabled={updating.size > 0} onClick={applyAllUpdates}>
                        {t("instance.updateAll")}
                      </button>
                    </div>
                    {updates!.map((u) => (
                      <div key={u.file_name} className="bg-panel-2 shadow-sunken flex items-center gap-[10px] py-[6px] px-[8px] rounded-none">
                        <div className="flex-1 min-w-0">
                          <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">{u.name}</div>
                          <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                            {(u.current_version ?? t("instance.currentVersion")) + " → " + u.new_version}
                          </div>
                        </div>
                        <button
                          className={INSTALL_BTN}
                          disabled={updating.has(u.file_name)}
                          onClick={() => applyUpdate(u)}
                        >
                          {updating.has(u.file_name) ? t("instance.updating") : t("instance.update")}
                        </button>
                      </div>
                    ))}
                  </div>
                )}

                {/* 已安装 mod 列表 */}
                {modsLoading ? (
                  <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
                    <Spinner size={16} /> {t("instance.scanningMods")}
                  </div>
                ) : (mods ?? []).length > 0 ? (
                  <div className="flex flex-col gap-[6px]">
                    {mods!.map((m) => (
                      <div
                        key={m.file_name}
                        className={clsx("flex items-center gap-[10px] py-[8px] px-[10px] rounded-none bg-panel-2", {
                          "opacity-55": !m.enabled,
                        })}
                      >
                        <div className="flex-1 min-w-0">
                          <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">{m.name}</div>
                          <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                            {[m.version, m.loader, m.file_name].filter(Boolean).join(" · ")}
                          </div>
                        </div>
                        <div className="flex items-center shrink-0">
                          <Toggle checked={m.enabled} onChange={(v) => toggleMod(m, v)} title={t("instance.enable")} />
                        </div>
                        <button className={DEL_BTN} onClick={() => setConfirmDelMod(m)}>
                          {t("instance.delete")}
                        </button>
                      </div>
                    ))}
                  </div>
                ) : modsError ? (
                  <ErrorState compact message={t("instance.modListError")} onRetry={() => void refetchMods()} />
                ) : (
                  <div className="flex flex-col items-center justify-center gap-[12px] py-[40px] text-center">
                    <div className="text-muted text-[13px]">{t("instance.noMods")}</div>
                    <button className={ACCENT_BTN} onClick={startBrowse}>
                      {t("instance.addMod")}
                    </button>
                  </div>
                )}
              </>
            )}
          </div>
        )}

        {/* ---- 资源包 / 光影 / 数据包 ---- */}
        {/* keep-alive:三个 pack 标签各自独立常驻,browse 只对当前激活的 pack 标签生效。 */}
        {visited.has("resource_pack") && props.instance && (
          <div className={clsx({ hidden: tab !== "resource_pack" })}>
            <PacksPanel
              instance={props.instance}
              kind="resource_pack"
              searchKind="resourcepack"
              emptyHint={t("instance.emptyResourcePack")}
              tick={importTick}
              browse={tab === "resource_pack" && browsing}
              onBrowse={setBrowsing}
            />
          </div>
        )}
        {visited.has("shader") && props.instance && (
          <div className={clsx({ hidden: tab !== "shader" })}>
            <PacksPanel
              instance={props.instance}
              kind="shader"
              searchKind="shader"
              emptyHint={t("instance.emptyShader")}
              tick={importTick}
              browse={tab === "shader" && browsing}
              onBrowse={setBrowsing}
            />
          </div>
        )}
        {visited.has("datapack") && props.instance && (
          <div className={clsx({ hidden: tab !== "datapack" })}>
            <PacksPanel
              instance={props.instance}
              kind="datapack"
              searchKind="datapack"
              emptyHint={t("instance.emptyDatapack")}
              tick={importTick}
              browse={tab === "datapack" && browsing}
              onBrowse={setBrowsing}
            />
          </div>
        )}

        {/* ---- 存档 ---- */}
        {visited.has("worlds") && props.instance && (
          <div className={clsx({ hidden: tab !== "worlds" })}>
            <WorldsPanel instance={props.instance} tick={worldTick} />
          </div>
        )}

        {/* ---- 多人服务器(servers.dat) ---- */}
        {visited.has("servers") && props.instance && (
          <div className={clsx("h-full min-h-0", { hidden: tab !== "servers" })}>
            <ServersPanel instance={props.instance} />
          </div>
        )}

        {/* ---- 截图 ---- */}
        {visited.has("screenshots") && props.instance && (
          <div className={clsx({ hidden: tab !== "screenshots" })}>
            <ScreenshotsPanel instance={props.instance} />
          </div>
        )}
      </div>

      {/* 内嵌模式(实例详情页)不渲染底部栏:复制实例移到详情页头部 ⋮ 菜单,完成本就不显示。 */}
      {!props.embedded && (
        <div className="flex justify-between items-center px-[20px] py-[14px] border-t border-titlebar">
          <Button variant="ghost" disabled={copying || !props.instance} onClick={copyInstance}>
            {copying ? t("instance.copying") : t("instance.copyInstance")}
          </Button>
          <Button variant="ghost" onClick={() => props.onClose?.()}>
            {t("instance.done")}
          </Button>
        </div>
      )}

      <Dialog
        open={confirmDelMod !== null}
        onClose={() => setConfirmDelMod(null)}
        label={t("instance.deleteMod")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <div className="text-[15px] font-semibold text-fg break-words">
            {t("instance.deleteModConfirm", { name: confirmDelMod?.name ?? "" })}
          </div>
          <div className="text-[13px] text-muted leading-[1.6]">{t("instance.deleteModBody")}</div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDelMod(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const m = confirmDelMod;
                setConfirmDelMod(null);
                if (m) void removeMod(m);
              }}
            >
              {t("instance.delete")}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );

  // 内嵌模式直接铺在父容器;否则套 Dialog 作模态。
  return props.embedded ? (
    body
  ) : (
    <Dialog
      open={props.open}
      onClose={() => props.onClose?.()}
      label={t("instance.instanceManage")}
      contentClass="w-[520px] max-w-[calc(100vw-48px)] rounded-none overflow-hidden"
    >
      {body}
    </Dialog>
  );
}

export default InstanceManageDialog;
