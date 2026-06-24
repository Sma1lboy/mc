import {
  Component,
  createSignal,
  createResource,
  createEffect,
  on,
  onMount,
  onCleanup,
  For,
  Show,
} from "solid-js";
import { createStore } from "solid-js/store";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Dialog } from "./Dialog";
import { InstanceIcon } from "./InstanceIcon";
import Lightbox from "./Lightbox";
import ServersPanel from "./ServersPanel";
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
import { activeRoot, openInstance, isRunning } from "../store";
import { t } from "../i18n";
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
const ScreenshotTile: Component<{
  info: ScreenshotInfo;
  url?: string;
  failed?: boolean;
  onVisible: () => void;
  onOpen: () => void;
  onRetry: () => void;
  onDelete: (e: MouseEvent) => void;
}> = (props) => {
  let el: HTMLDivElement | undefined;
  onMount(() => {
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          props.onVisible();
          io.disconnect();
        }
      },
      { rootMargin: "120px" },
    );
    if (el) io.observe(el);
    onCleanup(() => io.disconnect());
  });

  return (
    <div
      ref={el}
      class="group relative aspect-video rounded-none overflow-hidden bg-panel-2 cursor-pointer"
      onClick={props.onOpen}
    >
      <Show
        when={props.url}
        fallback={
          <Show
            when={props.failed}
            fallback={
              <div class="w-full h-full grid place-items-center">
                <Spinner size={16} />
              </div>
            }
          >
            {/* 读图失败:给可重试的占位,而不是永远转圈。 */}
            <button
              class="w-full h-full grid place-items-center gap-[2px] text-[11px] text-muted bg-panel-2 cursor-pointer hover:text-fg"
              onClick={(e) => {
                e.stopPropagation();
                props.onRetry();
              }}
              title={t("instance.reload")}
            >
              <span>{t("instance.loadFailed")}</span>
              <span class="text-[10px] underline">{t("instance.clickRetry")}</span>
            </button>
          </Show>
        }
      >
        <img src={props.url} alt={props.info.file_name} width="320" height="180" class="w-full h-full object-cover" />
      </Show>
      <button
        class="absolute top-[4px] right-[4px] opacity-0 group-hover:opacity-100 transition-opacity duration-150 text-[11px] text-white px-[6px] py-[2px] rounded-none bg-[rgba(0,0,0,0.55)] hover:bg-danger"
        onClick={props.onDelete}
      >
        {t("instance.delete")}
      </button>
    </div>
  );
};

/**
 * ScreenshotsPanel —— 实例截图栅格:懒加载缩略图、点开进灯箱、悬停删除。
 * 列表只取元数据,图片字节按需 read_screenshot;最多展示 SCREENSHOT_CAP 张(更多时提示)。
 */
const ScreenshotsPanel: Component<{ instance: InstanceSummary }> = (props) => {
  const [shots, { refetch }] = createResource(
    () => props.instance.id,
    (id) => api.instanceScreenshots(activeRoot(), id),
  );
  const capped = () => (shots() ?? []).slice(0, SCREENSHOT_CAP);
  const [urls, setUrls] = createStore<Record<string, string>>({});
  const [failed, setFailed] = createStore<Record<string, boolean>>({});
  const [lightbox, setLightbox] = createSignal<number | null>(null);

  async function loadUrl(fileName: string) {
    if (urls[fileName]) return;
    setFailed(fileName, false);
    try {
      const u = await api.readScreenshot(activeRoot(), props.instance.id, fileName);
      setUrls(fileName, u);
    } catch {
      // 单张读失败不致命:标记失败态,渲染可重试占位,不让缩略图永远转圈。
      setFailed(fileName, true);
    }
  }

  async function remove(s: ScreenshotInfo, e: MouseEvent) {
    e.stopPropagation(); // 别触发打开灯箱。
    try {
      await api.deleteScreenshot(activeRoot(), props.instance.id, s.file_name);
      toast({ type: "success", message: t("instance.deletedScreenshot") });
      refetch();
    } catch (err) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(err) }) });
    }
  }

  const lightboxImages = () =>
    capped().map((s) => ({ url: urls[s.file_name] ?? "", title: s.file_name }));

  // 打开/切换灯箱时确保目标图及左右相邻图已加载(缩略图可能还没滚动到、未触发懒加载),
  // 避免主图/缩略图条出现空白或裂图。
  function openLightbox(i: number) {
    const list = capped();
    for (const j of [i, i - 1, i + 1]) {
      const f = list[j]?.file_name;
      if (f) void loadUrl(f);
    }
    setLightbox(i);
  }

  return (
    <div class="flex flex-col gap-[8px]">
      <div class="flex items-center justify-between">
        <div class={LABEL}>{t("instance.screenshots")}</div>
        <button
          class={OPEN_BTN}
          onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "screenshots")}
        >
          {t("instance.openDir")}
        </button>
      </div>

      <Show when={(shots() ?? []).length > SCREENSHOT_CAP}>
        <div class="text-[11px] text-muted">
          {t("instance.screenshotCapNote", { total: shots()!.length, cap: SCREENSHOT_CAP })}
        </div>
      </Show>

      <Show
        when={!shots.loading}
        fallback={
          <div class="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
            <Spinner size={16} /> {t("instance.scanningScreenshots")}
          </div>
        }
      >
        <Show
          when={capped().length > 0}
          fallback={
            shots.error
              ? <ErrorState compact message={t("instance.screenshotLoadError")} onRetry={() => void refetch()} />
              : <div class="text-muted text-[13px] py-[12px]">{t("instance.noScreenshots")}</div>
          }
        >
          <div class="grid grid-cols-3 gap-[8px]">
            <For each={capped()}>
              {(s, i) => (
                <ScreenshotTile
                  info={s}
                  url={urls[s.file_name]}
                  failed={failed[s.file_name]}
                  onVisible={() => loadUrl(s.file_name)}
                  onOpen={() => openLightbox(i())}
                  onRetry={() => loadUrl(s.file_name)}
                  onDelete={(e) => remove(s, e)}
                />
              )}
            </For>
          </div>
        </Show>
      </Show>

      <Show when={lightbox() !== null}>
        <Lightbox
          images={lightboxImages()}
          index={lightbox()!}
          onIndex={openLightbox}
          onClose={() => setLightbox(null)}
        />
      </Show>
    </div>
  );
};

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
const PacksPanel: Component<{
  instance: InstanceSummary;
  kind: PackKind;
  searchKind: ProjectKind;
  emptyHint: string;
  /** 外部导入计数:递增即触发重扫(拖拽导入后由父组件 bump)。 */
  tick?: number;
  /** 受控的「浏览/添加」模式(由父组件统一持有,用于隐藏详情页头部)。 */
  browse: boolean;
  onBrowse: (v: boolean) => void;
}> = (props) => {
  // 数据包逐存档生效:落到 saves/<world>/datapacks。其它包类型无 world 概念。
  const isDatapack = () => props.kind === "datapack";
  const [worlds] = createResource(
    () => (isDatapack() ? props.instance.id : false),
    (id) => api.instanceWorlds(activeRoot(), id as string),
  );
  const [world, setWorld] = createSignal<string | null>(null);
  // 默认选中第一个存档(按上次游玩排序);存档变化后若当前选中已失效则回退。
  createEffect(() => {
    if (!isDatapack()) return;
    const list = worlds() ?? [];
    if (list.length === 0) {
      setWorld(null);
    } else if (!world() || !list.some((w) => w.folder === world())) {
      setWorld(list[0].folder);
    }
  });
  const worldArg = () => (isDatapack() ? world() : null);

  const [packs, { refetch }] = createResource(
    () => [props.instance.id, props.kind, props.tick ?? 0, worldArg()] as const,
    ([id, kind, , w]) => api.instancePacks(activeRoot(), id, kind, w),
  );

  const [detail, setDetail] = createSignal<ModpackHit | null>(null);
  // 详情页对应的来源平台(随 onOpenDetail 一起带过来),决定详情里取版本/安装走哪个 provider。
  const [detailProvider, setDetailProvider] = createSignal<ContentProvider>("modrinth");
  // 后台并行安装:正在安装的 project_id 集合(不阻塞其它行)。
  const [installing, setInstalling] = createSignal<Set<string>>(new Set());
  // 本次浏览已添加的 project_id:行按钮即时变「已添加」。
  const [added, setAdded] = createSignal<Set<string>>(new Set());
  // 删除资源包前确认(删除是破坏性的,与存档删除一致)。
  const [confirmDel, setConfirmDel] = createSignal<PackInfo | null>(null);
  const startBrowse = () => {
    setAdded(new Set<string>());
    props.onBrowse(true);
  };

  // 行内「下载」:直接装最新兼容版(资源包/光影/数据包不分加载器),后台并行不阻塞其它行。
  async function install(projectId: string, title: string, provider: ContentProvider = "modrinth") {
    if (installing().has(projectId)) return;
    setInstalling((s) => new Set(s).add(projectId));
    try {
      const report = await api.installPack(
        activeRoot(),
        props.instance.id,
        props.kind,
        projectId,
        props.instance.mc_version,
        worldArg(),
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
      await api.setPackEnabled(activeRoot(), props.instance.id, props.kind, p.file_name, enabled, worldArg());
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.opFailed", { err: String(e) }) });
    }
  }

  async function remove(p: PackInfo) {
    try {
      await api.deletePack(activeRoot(), props.instance.id, props.kind, p.file_name, worldArg());
      toast({ type: "success", message: t("instance.deletedFile", { file: p.file_name }) });
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(e) }) });
    }
  }

  return (
    <div class="flex flex-col gap-[8px]">
      {/* 数据包目标存档选择器:数据包是逐存档生效的,必须先选一个存档。 */}
      <Show when={isDatapack()}>
        <Show
          when={(worlds() ?? []).length > 0}
          fallback={
            <div class="text-[12px] leading-[1.7] text-muted py-[4px]">
              {t("instance.datapackNoWorlds")}
            </div>
          }
        >
          <label class="flex items-center gap-[8px] text-[12px] text-muted">
            <span class="shrink-0">{t("instance.targetWorld")}</span>
            <Select
              class="flex-1 !min-w-0"
              value={world() ?? ""}
              onChange={(v) => setWorld(v)}
              options={(worlds() ?? []).map((w) => ({ value: w.folder, label: w.name || w.folder }))}
            />
          </label>
        </Show>
      </Show>

      <Show
        when={props.browse}
        fallback={
          <>
            {/* 默认:「已安装」标题行,右侧聚拢动作(打开目录 + 紧凑「添加」)。 */}
            <div class="flex items-center justify-between">
              <div class={LABEL}>{t("instance.installedTitle")}</div>
              <div class="flex items-center gap-[6px]">
                <button
                  class={OPEN_BTN}
                  onClick={() =>
                    openInstanceSubdir(
                      activeRoot(),
                      props.instance.id,
                      props.kind === "resource_pack"
                        ? "resourcepacks"
                        : props.kind === "shader"
                          ? "shaderpacks"
                          : world()
                            ? `saves/${world()}/datapacks`
                            : "datapacks",
                    )
                  }
                >
                  {t("instance.openDir")}
                </button>
                <button
                  class="shrink-0 h-[28px] px-[10px] rounded-none bg-accent text-white shadow-raised text-[12px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed"
                  onClick={startBrowse}
                >
                  {t("instance.add")}
                </button>
              </div>
            </div>

            <Show
              when={!packs.loading}
              fallback={
                <div class="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
                  <Spinner size={16} /> {t("instance.scanning")}
                </div>
              }
            >
              <Show
                when={(packs() ?? []).length > 0}
                fallback={
                  packs.error ? (
                    <ErrorState compact message={t("instance.loadFailedShort")} onRetry={() => void refetch()} />
                  ) : (
                    <div class="flex flex-col items-center justify-center gap-[12px] py-[40px] text-center">
                      <div class="text-muted text-[13px]">{props.emptyHint}</div>
                      <button
                        class={ACCENT_BTN}
                        onClick={startBrowse}
                      >
                        {t("instance.add")}
                      </button>
                    </div>
                  )
                }
              >
                <div class="flex flex-col gap-[6px]">
                  <For each={packs()}>
                    {(p) => (
                      <div
                        class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-none bg-panel-2"
                        classList={{ "opacity-55": !p.enabled }}
                      >
                        <div class="flex-1 min-w-0">
                          <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                            {p.file_name.replace(/\.disabled$/, "")}
                          </div>
                          <div class="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                            {[p.description, fmtSize(p.size)].filter(Boolean).join(" · ")}
                          </div>
                        </div>
                        <div class="flex items-center shrink-0">
                          <Toggle checked={p.enabled} onChange={(v) => toggle(p, v)} title={t("instance.enable")} />
                        </div>
                        <button class={DEL_BTN} onClick={() => setConfirmDel(p)}>
                          {t("instance.delete")}
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </>
        }
      >
        {/* 浏览模式 = 复用探索页:搜索列表 →(点进)详情安装,装完回到已安装。 */}
        <Show
          when={detail()}
          fallback={
            <>
              <button
                class="self-start inline-flex items-center gap-[4px] h-[28px] px-[10px] rounded-none border-none bg-transparent text-muted text-[12px] cursor-pointer transition-colors duration-150 hover:bg-panel-3 hover:text-fg"
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
                onOpenDetail={(hit, provider) => { setDetail(hit); setDetailProvider(provider); }}
                onAdd={(hit, provider) => install(hit.id, hit.title, provider)}
                addingIds={installing()}
                addedIds={added()}
                disabledReason={
                  isDatapack() ? (() => (worldArg() ? null : t("instance.selectTargetWorldFirst"))) : undefined
                }
                autofocus
                onEscape={() => props.onBrowse(false)}
                placeholder={t("instance.searchModrinth", { version: props.instance.mc_version })}
              />
            </>
          }
        >
          {(d) => (
            <ProjectInstallDetail
              hit={d()}
              kind={props.searchKind as Exclude<ProjectKind, "modpack">}
              provider={detailProvider()}
              lockedInstance={props.instance}
              onBack={() => setDetail(null)}
              onInstalled={() => {
                refetch();
                const d = detail();
                if (d) setAdded((s) => new Set(s).add(d.id));
              }}
            />
          )}
        </Show>
      </Show>

      <Dialog
        open={confirmDel() !== null}
        onClose={() => setConfirmDel(null)}
        label={t("instance.deleteResourcePack")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-fg break-words">
            {t("instance.deleteFileConfirm", { file: confirmDel()?.file_name.replace(/\.disabled$/, "") ?? "" })}
          </div>
          <div class="text-[13px] text-muted leading-[1.6]">{t("instance.deleteFileBody")}</div>
          <div class="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDel(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const p = confirmDel();
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
};

/**
 * WorldsPanel —— 存档世界列表 + 备份(导出 zip)/ 重命名(改显示名)/ 删除(走回收站)。
 */
const WorldsPanel: Component<{ instance: InstanceSummary; tick?: number }> = (props) => {
  const [worlds, { refetch }] = createResource(
    () => [props.instance.id, props.tick ?? 0] as const,
    ([id]) => api.instanceWorlds(activeRoot(), id),
  );

  // 行内重命名:正在编辑的世界 folder + 草稿名。
  const [editing, setEditing] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal<string | null>(null);
  const [importing, setImporting] = createSignal(false);
  // 删除存档前确认(存档含游玩进度,删除是破坏性的)。
  const [confirmDel, setConfirmDel] = createSignal<WorldInfo | null>(null);

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
    // Escape 也先 setEditing(null) 再触发 onBlur。两种情况此时 editing() 已不是本行,
    // 直接返回 —— 避免重复重命名/重复 toast,以及「Escape 反而保存」。
    if (editing() !== w.folder) return;
    const name = draft().trim();
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

  const MODE_LABEL = (): Record<string, string> => ({
    survival: t("instance.modeSurvival"),
    creative: t("instance.modeCreative"),
    adventure: t("instance.modeAdventure"),
    spectator: t("instance.modeSpectator"),
    unknown: t("instance.modeUnknown"),
  });

  return (
    <div class="flex flex-col gap-[8px]">
      <div class="flex items-center justify-between">
        <div class={LABEL}>{t("instance.worldsListTitle")}</div>
        <div class="flex items-center gap-[4px]">
          <button
            class={OPEN_BTN}
            onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "saves")}
          >
            {t("instance.openDir")}
          </button>
          <button
            class="text-[12px] text-accent px-[8px] py-[3px] rounded-none cursor-pointer hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
            disabled={importing()}
            onClick={importZip}
          >
            {importing() ? t("instance.importingWorld") : t("instance.importWorld")}
          </button>
        </div>
      </div>

      <Show
        when={!worlds.loading}
        fallback={
          <div class="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
            <Spinner size={16} /> {t("instance.scanningWorlds")}
          </div>
        }
      >
      <Show
        when={(worlds() ?? []).length > 0}
        fallback={
          worlds.error
            ? <ErrorState compact message={t("instance.worldsLoadError")} onRetry={() => void refetch()} />
            : <div class="text-muted text-[13px] py-[12px]">{t("instance.noWorlds")}</div>
        }
      >
        <div class="flex flex-col gap-[6px]">
          <For each={worlds()}>
            {(w) => (
              <div class="bg-panel-2 shadow-sunken flex items-center gap-[10px] py-[8px] px-[10px] rounded-none">
                <div class="flex-1 min-w-0">
                  <Show
                    when={editing() === w.folder}
                    fallback={
                      <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                        {w.name}
                      </div>
                    }
                  >
                    <input
                      class={`${FIELD} h-[26px] w-full text-[12px]`}
                      ref={(el) => queueMicrotask(() => el.focus())}
                      name="worldName"
                      autocomplete="off"
                      spellcheck={false}
                      value={draft()}
                      onInput={(e) => setDraft(e.currentTarget.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") commitRename(w);
                        else if (e.key === "Escape") setEditing(null);
                      }}
                      onBlur={() => commitRename(w)}
                    />
                  </Show>
                  <div class="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                    {[
                      MODE_LABEL()[w.game_mode] ?? w.game_mode,
                      fmtSize(w.size_bytes),
                      w.seed != null ? t("instance.seed", { seed: w.seed }) : null,
                      w.folder,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </div>
                </div>
                <button
                  class="shrink-0 text-[12px] text-muted px-[8px] py-[4px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
                  disabled={busy() === w.folder}
                  onClick={() => backup(w)}
                >
                  {busy() === w.folder ? t("instance.backingUp") : t("instance.backup")}
                </button>
                <button
                  class="shrink-0 text-[12px] text-muted px-[8px] py-[4px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2"
                  onClick={() => startRename(w)}
                >
                  {t("instance.rename")}
                </button>
                <button class={DEL_BTN} onClick={() => setConfirmDel(w)}>
                  {t("instance.delete")}
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>
      </Show>

      <Dialog
        open={confirmDel() !== null}
        onClose={() => setConfirmDel(null)}
        label={t("instance.deleteWorld")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-fg">{t("instance.deleteWorldConfirm", { name: confirmDel()?.name ?? "" })}</div>
          <div class="text-[13px] text-muted leading-[1.6]">{t("instance.deleteWorldBody")}</div>
          <div class="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDel(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const w = confirmDel();
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
};

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
const AddLoaderPanel: Component<{
  instance: InstanceSummary;
  onAdded: (newId: string) => void;
}> = (props) => {
  const [loader, setLoader] = createSignal("fabric");
  const [version, setVersion] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [progress, setProgress] = createSignal("");
  const needsVersion = () => loader() === "forge" || loader() === "neoforge";

  const off = onInstallProgress((p) => {
    if (!busy()) return;
    setProgress(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage);
  });
  onCleanup(off);

  async function add() {
    if (busy()) return;
    if (needsVersion() && !version().trim()) {
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
        loader(),
        needsVersion() ? version().trim() : null,
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
    <div class="flex flex-col gap-[10px] py-[4px]">
      <div class="text-[13px] text-muted leading-[1.6]">
        {t("instance.addLoaderIntro")}
      </div>
      <div class="flex items-center gap-[8px]">
        <Select value={loader()} onChange={setLoader} options={LOADER_OPTS} />
        <Show when={needsVersion()}>
          <input
            class={`${FIELD} flex-1`}
            placeholder={loader() === "forge" ? t("instance.forgeBuildPlaceholder") : t("instance.neoforgeVersionPlaceholder")}
            value={version()}
            onInput={(e) => setVersion(e.currentTarget.value)}
          />
        </Show>
        <button
          class="shrink-0 h-[34px] px-[14px] rounded-none bg-accent text-white shadow-raised text-[13px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed disabled:opacity-50 disabled:cursor-default"
          disabled={busy()}
          onClick={add}
        >
          {busy() ? t("instance.installing") : t("instance.addLoader")}
        </button>
      </div>
      <Show when={busy() && progress()}>
        <div class="flex items-center gap-[8px] text-accent text-[12px]">
          <Spinner size={14} /> {progress()}
        </div>
      </Show>
    </div>
  );
};

export const InstanceManageDialog: Component<{
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
}> = (props) => {
  const [internalTab, setInternalTab] = createSignal<InstanceManageTab>("settings");
  const [cfg, setCfg] = createSignal<InstanceConfig | null>(null);
  const [copying, setCopying] = createSignal(false);
  // 「浏览/添加」模式:任一内容标签点「+ 添加」即进入,占满内容区(复用探索页)。
  // 切换标签时复位;变化时通知外层(详情页隐藏头部 + 本组件隐藏 tab 条)。
  const [browsing, setBrowsing] = createSignal(false);
  createEffect(() => props.onBrowsingChange?.(browsing()));

  const tab = () => props.tab ?? internalTab();
  const setTab = (next: InstanceManageTab) => {
    setInternalTab(next);
    props.onTabChange?.(next);
  };
  const packKind = (): PackKind => {
    const cur = tab();
    return isPackTab(cur) ? cur : "resource_pack";
  };
  // 整合包来源(modrinth)→ 多一个「概览」标签并置于首位。
  const modpackSource = () => {
    const s = cfg()?.source;
    return s && s.provider === "modrinth" ? s : null;
  };
  const visibleTabs = (): { key: InstanceManageTab; label: string }[] =>
    modpackSource() ? [{ key: "overview", label: t("instance.tabOverview") }, ...TABS()] : TABS();

  // 是否「活动」(应加载数据 / 接受拖放):弹窗模式看 open,内嵌模式只要挂载即活动。
  const active = () => props.embedded || props.open;

  async function copyInstance() {
    const inst = props.instance;
    if (!inst) return;
    // 运行中复制会把正在写入的存档/文件拷成半截,复制副本可能损坏 —— 先要求停止。
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

  // 打开/切换实例时拉配置 + 复位到设置页;关闭时清空。
  createEffect(() => {
    const inst = props.instance;
    setUpdates(null); // 切换实例/开关时清掉上一个实例的更新检查结果。
    if (active() && inst) {
      setCfg(null);
      api
        .getInstanceConfig(activeRoot(), inst.id)
        .then((c) => {
          setCfg(c);
          // 整合包来源实例默认落在「概览」标签(仅非受控时)。
          if (props.tab === undefined)
            setInternalTab(c.source?.provider === "modrinth" ? "overview" : "settings");
        })
        .catch((e) => toast({ type: "error", message: t("instance.readConfigFailed", { err: String(e) }) }));
    } else if (!active()) {
      setCfg(null);
      setTab("settings");
    }
  });

  // Mods:仅在 Mods 标签 + 弹窗打开时拉取。
  const [mods, { refetch: refetchMods }] = createResource(
    () => (active() && props.instance && tab() === "mods" ? props.instance.id : false),
    (id) => api.instanceMods(activeRoot(), id as string),
  );

  // ---- 从 Modrinth 搜索并安装 ----
  // vanilla 实例没有加载器,搜 mod 无意义,这里把 loader 归一为 null(不限)。
  const searchLoader = () => {
    const l = props.instance?.loader;
    return l && l !== "vanilla" ? l : null;
  };
  const [modDetail, setModDetail] = createSignal<ModpackHit | null>(null);
  const [modDetailProvider, setModDetailProvider] = createSignal<ContentProvider>("modrinth");
  // 后台并行安装:正在安装的 project_id 集合(不阻塞其它行)。
  const [installing, setInstalling] = createSignal<Set<string>>(new Set());
  // 本次浏览已添加的 mod project_id:行按钮即时变「已添加」,无需返回已安装确认。
  const [addedMods, setAddedMods] = createSignal<Set<string>>(new Set());
  // 删除 mod 前确认(删除是破坏性的,与存档/资源包删除一致)。
  const [confirmDelMod, setConfirmDelMod] = createSignal<ModInfo | null>(null);
  const startBrowse = () => {
    setAddedMods(new Set<string>());
    setBrowsing(true);
  };
  // 切换标签(含外层受控切换)即退出浏览/添加模式并清掉详情,避免浏览态串到别的标签。
  createEffect(on(tab, () => {
    setBrowsing(false);
    setModDetail(null);
  }, { defer: true }));

  // 行内「下载」:直接装最新兼容版(解析依赖),不进详情;后台并行,不阻塞其它行。
  async function installHit(projectId: string, title: string, provider: ContentProvider = "modrinth") {
    const inst = props.instance;
    if (!inst || installing().has(projectId)) return;
    setInstalling((s) => new Set(s).add(projectId));
    try {
      const report = await api.installMod(activeRoot(), inst.id, projectId, inst.mc_version, searchLoader() ?? "", provider);
      if ((report.blocked?.length ?? 0) > 0) {
        toast({ type: "warn", message: t("instance.blockedManual", { count: report.blocked!.length }) });
      } else {
        if (report.installed.length === 0 && report.unresolved.length === 0) {
          toast({ type: "info", message: t("instance.modExists", { title }) });
        } else {
          const parts = [t("instance.modInstalledCount", { n: report.installed.length })];
          if (report.unresolved.length > 0) parts.push(t("instance.modUnresolvedCount", { n: report.unresolved.length }));
          toast({ type: report.unresolved.length > 0 ? "warn" : "success", message: t("instance.modInstallResult", { title, parts: parts.join(",") }) });
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
  const [updates, setUpdates] = createSignal<ModUpdate[] | null>(null);
  const [checking, setChecking] = createSignal(false);
  // 后台并行更新:正在更新的文件集合(不阻塞其它行/全部更新串行)。
  const [updating, setUpdating] = createSignal<Set<string>>(new Set());

  async function checkUpdates() {
    const inst = props.instance;
    if (!inst) return;
    setChecking(true);
    try {
      const list = await api.checkModUpdates(
        activeRoot(),
        inst.id,
        inst.mc_version,
        searchLoader() ?? "",
      );
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
    if (!inst || updating().has(u.file_name)) return;
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
    await Promise.all((updates() ?? []).map((u) => applyUpdate(u)));
  }

  // ---- 拖拽导入 ----
  // Tauri 启用了原生文件拖放,HTML5 ondrop 不触发,改用 webview 的 onDragDropEvent。
  // 整个弹窗作为拖放区,目标类型由当前标签决定。
  const [dragOver, setDragOver] = createSignal(false);
  const [dropping, setDropping] = createSignal(false);
  const [importTick, setImportTick] = createSignal(0);
  const [worldTick, setWorldTick] = createSignal(0);

  /** 当前标签接受拖拽吗(设置标签不接受)。 */
  function dropAccepted(): boolean {
    return tab() === "mods" || isPackTab(tab()) || tab() === "worlds";
  }

  /** mods/资源包/光影/数据包的导入目标类型(存档走单独的 zip 导入命令,这里返回 null)。 */
  function resourceTarget(): string | null {
    if (tab() === "mods") return "mod";
    if (isPackTab(tab())) return tab() === "resource_pack" ? "resourcepack" : tab();
    return null;
  }

  async function handleDrop(paths: string[]) {
    const inst = props.instance;
    if (!inst || !dropAccepted()) {
      toast({ type: "info", message: t("instance.dropHint") });
      return;
    }
    const cur = tab();
    setDropping(true);
    try {
      // 并行导入(串行会让拖入多个大文件逐个卡住);用 allSettled 汇总成败。
      const results = await Promise.allSettled(
        paths.map((path) =>
          cur === "worlds"
            ? api.importWorldZip(activeRoot(), inst.id, path)
            : api.importLocalResource(activeRoot(), inst.id, resourceTarget()!, path, null),
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

  createEffect(() => {
    if (!active()) return;
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      if (!active()) return;
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") setDragOver(true);
      else if (p.type === "leave") setDragOver(false);
      else if (p.type === "drop") {
        setDragOver(false);
        void handleDrop(p.paths);
      }
    });
    onCleanup(() => void unlisten.then((f) => f()));
  });

  function patch(p: Partial<InstanceConfig>) {
    const cur = cfg();
    const inst = props.instance;
    if (!cur || !inst) return;
    const next = { ...cur, ...p };
    setCfg(next);
    void api
      .setInstanceConfig(activeRoot(), inst.id, next)
      .then(() => props.onChanged?.())
      .catch((e) => toast({ type: "error", message: t("instance.saveFailed", { err: String(e) }) }));
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

  const body = (
      <div
        class="relative flex flex-col transition-shadow duration-150"
        classList={{
          "max-h-[calc(100vh-100px)]": !props.embedded,
          "h-full": props.embedded,
          "ring-2 ring-inset ring-accent": dragOver(),
        }}
      >
        <Show when={dragOver() && dropAccepted()}>
          <div class="absolute inset-0 z-10 grid place-items-center bg-window/85 pointer-events-none">
            <div class="text-[14px] text-accent font-semibold">{t("instance.dropToImport")}</div>
          </div>
        </Show>
        <Show when={dropping()}>
          <div class="absolute inset-0 z-10 grid place-items-center bg-window/85">
            <div class="flex items-center gap-[10px] text-[14px] text-fg font-semibold">
              <Spinner size={18} /> {t("instance.importingOverlay")}
            </div>
          </div>
        </Show>
        <Show when={!props.embedded}>
          <Heading size="sub" class="px-[20px] pt-[18px]">
            {props.instance?.name || props.instance?.id}
          </Heading>
        </Show>

        <Show when={!props.hideTabs && !browsing()}>
          <div class="shrink-0 flex gap-[4px] px-[16px] border-b border-titlebar mt-[10px] overflow-x-auto">
            <For each={visibleTabs()}>
              {(item) => (
                <button
                  class={`${TAB} whitespace-nowrap ${tab() === item.key ? TAB_ACTIVE : ""}`}
                  onClick={() => setTab(item.key)}
                >
                  {item.label}
                </button>
              )}
            </For>
          </div>
        </Show>

        <div class="flex-1 min-h-0 p-[20px] flex flex-col gap-[14px] overflow-y-auto">
          {/* ---- 概览(整合包来源)---- */}
          <Show when={tab() === "overview" && modpackSource()}>
            {(s) => <ModpackOverview projectId={s().project_id} />}
          </Show>

          {/* ---- 设置 ---- */}
          <Show when={tab() === "settings"}>
            <Show
              when={cfg()}
              fallback={
                <div class="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
                  <Spinner size={16} /> {t("instance.readingConfig")}
                </div>
              }
            >
              {(c) => (
                <>
                  <div class="flex items-center gap-[12px]">
                    <div class="w-[56px] h-[56px] rounded-none overflow-hidden bg-panel-2 shrink-0 select-none">
                      <InstanceIcon name={props.instance?.name || props.instance?.id} icon={props.instance?.icon ?? undefined} />
                    </div>
                    <div class="flex flex-col gap-[5px]">
                      <span class={LABEL}>{t("instance.instanceIcon")}</span>
                      <button
                        class="h-[30px] px-[12px] shadow-raised rounded-none bg-panel-3 text-fg text-[12px] cursor-pointer transition-[box-shadow,filter] duration-[var(--dur)] ease-app hover:brightness-110 active:shadow-pressed w-fit"
                        onClick={pickIcon}
                      >
                        {t("instance.changeIcon")}
                      </button>
                    </div>
                  </div>

                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>{t("instance.name")}</span>
                    <input
                      class={FIELD}
                      value={c().name ?? ""}
                      onChange={(e) => patch({ name: e.currentTarget.value || null })}
                    />
                  </label>

                  <div class="flex flex-col gap-[5px]">
                    <span class={LABEL}>{t("instance.maxMemory", { mb: c().memory_mb ?? 0 })}</span>
                    <input
                      class="kb-range"
                      type="range"
                      min="512"
                      max="16384"
                      step="256"
                      value={c().memory_mb}
                      onInput={(e) => setCfg({ ...c(), memory_mb: +e.currentTarget.value })}
                      onChange={(e) => patch({ memory_mb: +e.currentTarget.value })}
                    />
                  </div>

                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>{t("instance.javaPath")}</span>
                    <input
                      class={FIELD}
                      placeholder={t("instance.javaPathPlaceholder")}
                      value={c().java_path ?? ""}
                      onChange={(e) => patch({ java_path: e.currentTarget.value || null })}
                    />
                  </label>

                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>{t("instance.extraJvmArgs")}</span>
                    <input
                      class={FIELD}
                      value={(c().jvm_args ?? []).join(" ")}
                      onChange={(e) =>
                        patch({ jvm_args: e.currentTarget.value.split(/\s+/).filter(Boolean) })
                      }
                    />
                  </label>

                  <div class="flex gap-[12px]">
                    <label class="flex-1 flex flex-col gap-[5px]">
                      <span class={LABEL}>{t("instance.windowWidth")}</span>
                      <input
                        class={FIELD}
                        type="number"
                        min="1"
                        max="7680"
                        placeholder={t("instance.defaultPlaceholder")}
                        value={c().width ?? ""}
                        onChange={(e) => {
                          const n = Math.floor(+e.currentTarget.value);
                          patch({ width: Number.isFinite(n) && n > 0 ? n : null });
                        }}
                      />
                    </label>
                    <label class="flex-1 flex flex-col gap-[5px]">
                      <span class={LABEL}>{t("instance.windowHeight")}</span>
                      <input
                        class={FIELD}
                        type="number"
                        min="1"
                        max="4320"
                        placeholder={t("instance.defaultPlaceholder")}
                        value={c().height ?? ""}
                        onChange={(e) => {
                          const n = Math.floor(+e.currentTarget.value);
                          patch({ height: Number.isFinite(n) && n > 0 ? n : null });
                        }}
                      />
                    </label>
                  </div>

                  <div class="flex items-center justify-between text-fg text-[13px]">
                    <span>{t("instance.fullscreenLaunch")}</span>
                    <Toggle checked={c().fullscreen ?? false} onChange={(v) => patch({ fullscreen: v })} title={t("instance.fullscreenLaunch")} />
                  </div>

                  <div class="pt-[4px]">
                    <button
                      class="h-[30px] px-[12px] shadow-raised rounded-none bg-panel-3 text-fg text-[12px] cursor-pointer transition-[box-shadow,filter] duration-[var(--dur)] ease-app hover:brightness-110 active:shadow-pressed"
                      onClick={() => props.instance && openInstanceDir(activeRoot(), props.instance.id)}
                    >
                      {t("instance.openGameDir")}
                    </button>
                  </div>
                </>
              )}
            </Show>
          </Show>

          {/* ---- Mods ---- */}
          <Show when={tab() === "mods"}>
            {/* 从 Modrinth 搜索并安装(按本实例的 MC 版本 + 加载器过滤)。
                搜索体验复用 <ContentBrowser>;「添加」装最新兼容版,点击行打开详情。 */}
            <div class="flex flex-col gap-[8px]">
              <Show
                when={searchLoader() !== null}
                fallback={
                  <AddLoaderPanel
                    instance={props.instance!}
                    onAdded={(newId) => {
                      props.onChanged?.();
                      if (newId !== props.instance!.id) openInstance(newId);
                    }}
                  />
                }
              >
                <Show
                  when={browsing()}
                  fallback={
                    <>
                      {/* 默认:「已安装」标题行,右侧聚拢动作(打开目录 + 检查更新 + 紧凑「添加」)。 */}
                      <div class="flex items-center justify-between">
                        <div class={LABEL}>{t("instance.installedTitle")}</div>
                        <div class="flex items-center gap-[6px]">
                          <button
                            class={OPEN_BTN}
                            onClick={() => openInstanceSubdir(activeRoot(), props.instance!.id, "mods")}
                          >
                            {t("instance.openDir")}
                          </button>
                          <button
                            class="text-[12px] text-accent px-[8px] py-[3px] rounded-none cursor-pointer hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
                            disabled={checking() || searchLoader() === null}
                            onClick={checkUpdates}
                          >
                            {checking() ? t("instance.checking") : t("instance.checkUpdates")}
                          </button>
                          <button
                            class="shrink-0 h-[28px] px-[10px] rounded-none bg-accent text-white shadow-raised text-[12px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed"
                            onClick={startBrowse}
                          >
                            {t("instance.add")}
                          </button>
                        </div>
                      </div>

                      {/* 可更新清单(检查后才出现) */}
                      <Show when={(updates() ?? []).length > 0}>
                        <div class="flex flex-col gap-[6px] rounded-none bg-panel-2 p-[8px]">
                          <div class="flex items-center justify-between">
                            <span class="text-[12px] text-fg font-semibold">
                              {t("instance.updatesAvailable", { n: updates()!.length })}
                            </span>
                            <button
                              class={INSTALL_BTN}
                              disabled={updating().size > 0}
                              onClick={applyAllUpdates}
                            >
                              {t("instance.updateAll")}
                            </button>
                          </div>
                          <For each={updates()}>
                            {(u) => (
                              <div class="bg-panel-2 shadow-sunken flex items-center gap-[10px] py-[6px] px-[8px] rounded-none">
                                <div class="flex-1 min-w-0">
                                  <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                                    {u.name}
                                  </div>
                                  <div class="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                                    {(u.current_version ?? t("instance.currentVersion")) + " → " + u.new_version}
                                  </div>
                                </div>
                                <button
                                  class={INSTALL_BTN}
                                  disabled={updating().has(u.file_name)}
                                  onClick={() => applyUpdate(u)}
                                >
                                  {updating().has(u.file_name) ? t("instance.updating") : t("instance.update")}
                                </button>
                              </div>
                            )}
                          </For>
                        </div>
                      </Show>

                      {/* 已安装 mod 列表 */}
                      <Show
                        when={!mods.loading}
                        fallback={
                          <div class="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
                            <Spinner size={16} /> {t("instance.scanningMods")}
                          </div>
                        }
                      >
                        <Show
                          when={(mods() ?? []).length > 0}
                          fallback={
                            mods.error ? (
                              <ErrorState compact message={t("instance.modListError")} onRetry={() => void refetchMods()} />
                            ) : (
                              <div class="flex flex-col items-center justify-center gap-[12px] py-[40px] text-center">
                                <div class="text-muted text-[13px]">{t("instance.noMods")}</div>
                                <button
                                  class={ACCENT_BTN}
                                  onClick={startBrowse}
                                >
                                  {t("instance.addMod")}
                                </button>
                              </div>
                            )
                          }
                        >
                          <div class="flex flex-col gap-[6px]">
                            <For each={mods()}>
                              {(m) => (
                                <div
                                  class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-none bg-panel-2"
                                  classList={{ "opacity-55": !m.enabled }}
                                >
                                  <div class="flex-1 min-w-0">
                                    <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                                      {m.name}
                                    </div>
                                    <div class="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                                      {[m.version, m.loader, m.file_name].filter(Boolean).join(" · ")}
                                    </div>
                                  </div>
                                  <div class="flex items-center shrink-0">
                                    <Toggle checked={m.enabled} onChange={(v) => toggleMod(m, v)} title={t("instance.enable")} />
                                  </div>
                                  <button class={DEL_BTN} onClick={() => setConfirmDelMod(m)}>
                                    {t("instance.delete")}
                                  </button>
                                </div>
                              )}
                            </For>
                          </div>
                        </Show>
                      </Show>
                    </>
                  }
                >
                  {/* 浏览模式 = 复用探索页:搜索列表 →(点进)详情安装,装完回到已安装。 */}
                  <Show
                    when={modDetail()}
                    fallback={
                      <>
                        <button
                          class="self-start inline-flex items-center gap-[4px] h-[28px] px-[10px] rounded-none border-none bg-transparent text-muted text-[12px] cursor-pointer transition-colors duration-150 hover:bg-panel-3 hover:text-fg"
                          onClick={() => setBrowsing(false)}
                        >
                          {t("instance.backToInstalled")}
                        </button>
                        <ContentBrowser
                          kind="mod"
                          mcVersion={props.instance?.mc_version ?? ""}
                          loader={searchLoader()}
                          onOpenDetail={(hit, provider) => { setModDetail(hit); setModDetailProvider(provider); }}
                          onAdd={(hit, provider) => installHit(hit.id, hit.title, provider)}
                          addingIds={installing()}
                          addedIds={addedMods()}
                          autofocus
                          onEscape={() => setBrowsing(false)}
                          placeholder={t("instance.searchModrinthMod", { version: props.instance?.mc_version ?? "", loader: searchLoader() ?? t("instance.noLoader") })}
                        />
                      </>
                    }
                  >
                    {(d) => (
                      <ProjectInstallDetail
                        hit={d()}
                        kind="mod"
                        provider={modDetailProvider()}
                        lockedInstance={props.instance!}
                        onBack={() => setModDetail(null)}
                        onInstalled={() => {
                          refetchMods();
                          setAddedMods((s) => new Set(s).add(d().id));
                        }}
                      />
                    )}
                  </Show>
                </Show>
              </Show>
            </div>
          </Show>

          {/* ---- 资源包 / 光影 / 数据包 ---- */}
          <Show when={isPackTab(tab()) && props.instance}>
            {(inst) => (
              <>
                <Show when={packKind() === "resource_pack"}>
                  <PacksPanel
                    instance={inst()}
                    kind="resource_pack"
                    searchKind="resourcepack"
                    emptyHint={t("instance.emptyResourcePack")}
                    tick={importTick()}
                    browse={browsing()}
                    onBrowse={setBrowsing}
                  />
                </Show>
                <Show when={packKind() === "shader"}>
                  <PacksPanel
                    instance={inst()}
                    kind="shader"
                    searchKind="shader"
                    emptyHint={t("instance.emptyShader")}
                    tick={importTick()}
                    browse={browsing()}
                    onBrowse={setBrowsing}
                  />
                </Show>
                <Show when={packKind() === "datapack"}>
                  <PacksPanel
                    instance={inst()}
                    kind="datapack"
                    searchKind="datapack"
                    emptyHint={t("instance.emptyDatapack")}
                    tick={importTick()}
                    browse={browsing()}
                    onBrowse={setBrowsing}
                  />
                </Show>
              </>
            )}
          </Show>

          {/* ---- 存档 ---- */}
          <Show when={tab() === "worlds" && props.instance}>
            {(inst) => <WorldsPanel instance={inst()} tick={worldTick()} />}
          </Show>

          {/* ---- 多人服务器(servers.dat) ---- */}
          <Show when={tab() === "servers" && props.instance}>
            {(inst) => <ServersPanel instance={inst()} />}
          </Show>

          {/* ---- 截图 ---- */}
          <Show when={tab() === "screenshots" && props.instance}>
            {(inst) => <ScreenshotsPanel instance={inst()} />}
          </Show>
        </div>

        {/* 内嵌模式(实例详情页)不渲染底部栏:复制实例移到详情页头部 ⋮ 菜单,完成本就不显示。 */}
        <Show when={!props.embedded}>
          <div class="flex justify-between items-center px-[20px] py-[14px] border-t border-titlebar">
            <Button variant="ghost" disabled={copying() || !props.instance} onClick={copyInstance}>
              {copying() ? t("instance.copying") : t("instance.copyInstance")}
            </Button>
            <Button variant="ghost" onClick={() => props.onClose?.()}>
              {t("instance.done")}
            </Button>
          </div>
        </Show>

        <Dialog
          open={confirmDelMod() !== null}
          onClose={() => setConfirmDelMod(null)}
          label={t("instance.deleteMod")}
          contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
        >
          <div class="p-[20px] flex flex-col gap-[14px]">
            <div class="text-[15px] font-semibold text-fg break-words">
              {t("instance.deleteModConfirm", { name: confirmDelMod()?.name ?? "" })}
            </div>
            <div class="text-[13px] text-muted leading-[1.6]">{t("instance.deleteModBody")}</div>
            <div class="flex justify-end gap-[10px]">
              <Button variant="ghost" onClick={() => setConfirmDelMod(null)}>
                {t("instance.cancel")}
              </Button>
              <Button
                variant="danger"
                onClick={() => {
                  const m = confirmDelMod();
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
};

export default InstanceManageDialog;
