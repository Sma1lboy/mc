import {
  Component,
  createSignal,
  createResource,
  createEffect,
  onMount,
  onCleanup,
  For,
  Show,
} from "solid-js";
import { createStore } from "solid-js/store";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Dialog } from "./Dialog";
import Lightbox from "./Lightbox";
import { ProjectDetailPanel } from "./ProjectDetailPanel";
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { openInstanceDir, openInstanceSubdir } from "../util/instanceActions";
import { activeRoot } from "../store";
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
  "h-[34px] px-[12px] rounded-ctl border border-n-6 bg-n-2 text-fg text-[13px] " +
  "transition-[border-color,box-shadow] duration-150 focus-visible:outline-none " +
  "focus-visible:border-a-4 focus-visible:ring-2 focus-visible:ring-a-5/25";
const LABEL = "text-[12px] text-dim";
const TAB =
  "px-[14px] py-[7px] text-[13px] font-semibold cursor-pointer border-b-2 border-b-transparent " +
  "text-n-6 hover:text-n-8 transition-colors duration-150";
const TAB_ACTIVE = "!text-a-4 !border-b-a-4";

export type InstanceManageTab =
  | "settings"
  | "mods"
  | "resource_pack"
  | "shader"
  | "datapack"
  | "worlds"
  | "screenshots";

const TABS: { key: InstanceManageTab; label: string }[] = [
  { key: "settings", label: "设置" },
  { key: "mods", label: "Mods" },
  { key: "resource_pack", label: "资源包" },
  { key: "shader", label: "光影" },
  { key: "datapack", label: "数据包" },
  { key: "worlds", label: "存档" },
  { key: "screenshots", label: "截图" },
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
      class="group relative aspect-video rounded-ctl overflow-hidden bg-n-3 cursor-pointer"
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
              class="w-full h-full grid place-items-center gap-[2px] text-[11px] text-dim bg-n-3 cursor-pointer hover:text-fg"
              onClick={(e) => {
                e.stopPropagation();
                props.onRetry();
              }}
              title="重新加载"
            >
              <span>加载失败</span>
              <span class="text-[10px] underline">点击重试</span>
            </button>
          </Show>
        }
      >
        <img src={props.url} alt={props.info.file_name} width="320" height="180" class="w-full h-full object-cover" />
      </Show>
      <button
        class="absolute top-[4px] right-[4px] opacity-0 group-hover:opacity-100 transition-opacity duration-150 text-[11px] text-white px-[6px] py-[2px] rounded-xs bg-[rgba(0,0,0,0.55)] hover:bg-[rgba(229,132,138,0.9)]"
        onClick={props.onDelete}
      >
        删除
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
      toast({ type: "success", message: "已删除截图" });
      refetch();
    } catch (err) {
      toast({ type: "error", message: `删除失败:${err}` });
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
        <div class={LABEL}>截图</div>
        <button
          class={OPEN_BTN}
          onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "screenshots")}
        >
          打开目录
        </button>
      </div>

      <Show when={(shots() ?? []).length > SCREENSHOT_CAP}>
        <div class="text-[11px] text-dim">
          共 {shots()!.length} 张,仅展示最新 {SCREENSHOT_CAP} 张(其余可在「打开目录」里查看)。
        </div>
      </Show>

      <Show
        when={!shots.loading}
        fallback={
          <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
            <Spinner size={16} /> 扫描截图…
          </div>
        }
      >
        <Show
          when={capped().length > 0}
          fallback={<div class="text-dim text-[13px] py-[12px]">该实例还没有截图。</div>}
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

const INSTALL_BTN =
  "shrink-0 h-[28px] px-[12px] rounded-ctl bg-a-4 text-white text-[12px] font-semibold cursor-pointer " +
  "transition-opacity duration-150 hover:opacity-90 disabled:opacity-50 disabled:cursor-default";
const DEL_BTN =
  "shrink-0 text-[12px] text-[#e5848a] px-[8px] py-[4px] rounded-xs cursor-pointer hover:bg-[rgba(229,132,138,0.14)]";
const OPEN_BTN =
  "shrink-0 text-[12px] text-dim px-[8px] py-[3px] rounded-xs cursor-pointer hover:text-fg hover:bg-a-4/10";

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

  const [query, setQuery] = createSignal("");
  const [debounced, setDebounced] = createSignal("");
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [detail, setDetail] = createSignal<{ id: string; title: string; icon?: string | null } | null>(null);
  let timer: number | undefined;
  function onInput(v: string) {
    setQuery(v);
    clearTimeout(timer);
    timer = window.setTimeout(() => setDebounced(v.trim()), 280);
  }
  onCleanup(() => clearTimeout(timer));

  const [hits] = createResource(
    () => (debounced() ? ([props.searchKind, debounced()] as const) : false),
    () => api.modrinthSearch(debounced(), props.searchKind, props.instance.mc_version, null),
  );

  async function install(projectId: string, title: string) {
    setInstalling(projectId);
    try {
      const file = await api.installPack(
        activeRoot(),
        props.instance.id,
        props.kind,
        projectId,
        props.instance.mc_version,
        worldArg(),
      );
      toast({ type: "success", message: `已安装 ${title}(${file})` });
      refetch();
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
    } finally {
      setInstalling(null);
    }
  }

  async function toggle(p: PackInfo, enabled: boolean) {
    try {
      await api.setPackEnabled(activeRoot(), props.instance.id, props.kind, p.file_name, enabled, worldArg());
      refetch();
    } catch (e) {
      toast({ type: "error", message: `操作失败:${e}` });
    }
  }

  async function remove(p: PackInfo) {
    try {
      await api.deletePack(activeRoot(), props.instance.id, props.kind, p.file_name, worldArg());
      toast({ type: "success", message: `已删除 ${p.file_name}` });
      refetch();
    } catch (e) {
      toast({ type: "error", message: `删除失败:${e}` });
    }
  }

  return (
    <div class="flex flex-col gap-[8px]">
      {/* 数据包目标存档选择器:数据包是逐存档生效的,必须先选一个存档。 */}
      <Show when={isDatapack()}>
        <Show
          when={(worlds() ?? []).length > 0}
          fallback={
            <div class="text-[12px] leading-[1.7] text-dim py-[4px]">
              这个实例还没有存档。数据包是按存档生效的(放进 <code>saves/&lt;存档&gt;/datapacks</code>),
              先在「存档」里创建/导入一个存档,或进游戏新建世界后再来安装。
            </div>
          }
        >
          <label class="flex items-center gap-[8px] text-[12px] text-dim">
            <span class="shrink-0">目标存档</span>
            <select
              class={`${FIELD} flex-1`}
              value={world() ?? ""}
              onChange={(e) => setWorld(e.currentTarget.value)}
            >
              <For each={worlds()}>
                {(w) => <option value={w.folder}>{w.name || w.folder}</option>}
              </For>
            </select>
          </label>
        </Show>
      </Show>

      <div class="relative">
        <input
          class={`${FIELD} w-full pr-[30px]`}
          placeholder={`搜索 Modrinth(${props.instance.mc_version})`}
          value={query()}
          onInput={(e) => onInput(e.currentTarget.value)}
        />
        <Show when={hits.loading}>
          <div class="absolute right-[10px] top-1/2 -translate-y-1/2">
            <Spinner size={14} />
          </div>
        </Show>
      </div>

      <Show when={debounced() && !hits.loading && (hits() ?? []).length === 0}>
        <div class="text-[12px] text-dim py-[4px]">没有匹配的结果。</div>
      </Show>

      <Show when={(hits() ?? []).length > 0}>
        <div class="flex flex-col gap-[6px] max-h-[180px] overflow-y-auto">
          <For each={hits()}>
            {(h) => (
              <div class="flex items-center gap-[10px] py-[7px] px-[10px] rounded-ctl bg-n-2 border border-n-3">
                <Show
                  when={h.icon_url}
                  fallback={<div class="w-[30px] h-[30px] rounded-xs bg-n-4 shrink-0" />}
                >
                  <img src={h.icon_url!} alt="" width="30" height="30" class="w-[30px] h-[30px] rounded-xs object-cover shrink-0" />
                </Show>
                <div class="flex-1 min-w-0">
                  <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                    {h.title}
                  </div>
                  <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                    {h.description}
                  </div>
                </div>
                <button
                  class={OPEN_BTN}
                  onClick={() => setDetail({ id: h.project_id, title: h.title, icon: h.icon_url })}
                >
                  详情
                </button>
                <button
                  class={INSTALL_BTN}
                  disabled={installing() !== null || (isDatapack() && !world())}
                  title={isDatapack() && !world() ? "先选择目标存档" : ""}
                  onClick={() => install(h.project_id, h.title)}
                >
                  {installing() === h.project_id ? "安装中…" : "安装"}
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>

      <div class="h-px bg-n-3 my-[2px]" />
      <div class="flex items-center justify-between">
        <div class={LABEL}>已安装</div>
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
          打开目录
        </button>
      </div>

      <Show
        when={!packs.loading}
        fallback={
          <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
            <Spinner size={16} /> 扫描中…
          </div>
        }
      >
        <Show
          when={(packs() ?? []).length > 0}
          fallback={<div class="text-dim text-[13px] py-[12px]">{props.emptyHint}</div>}
        >
          <div class="flex flex-col gap-[6px]">
            <For each={packs()}>
              {(p) => (
                <div
                  class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-ctl bg-n-3"
                  classList={{ "opacity-55": !p.enabled }}
                >
                  <div class="flex-1 min-w-0">
                    <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                      {p.file_name.replace(/\.disabled$/, "")}
                    </div>
                    <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                      {[p.description, fmtSize(p.size)].filter(Boolean).join(" · ")}
                    </div>
                  </div>
                  <label class="flex items-center gap-[5px] text-[11px] text-dim cursor-pointer shrink-0">
                    <input
                      type="checkbox"
                      class="w-[15px] h-[15px] accent-[var(--a-4)] cursor-pointer"
                      checked={p.enabled}
                      onChange={(e) => toggle(p, e.currentTarget.checked)}
                    />
                    启用
                  </label>
                  <button class={DEL_BTN} onClick={() => remove(p)}>
                    删除
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>

      <Show when={detail()}>
        {(d) => (
          <ProjectDetailPanel
            projectId={d().id}
            title={d().title}
            iconUrl={d().icon}
            target={props.searchKind}
            instanceId={props.instance.id}
            mcVersion={props.instance.mc_version}
            loader={null}
            world={worldArg()}
            onClose={() => setDetail(null)}
            onInstalled={() => refetch()}
          />
        )}
      </Show>
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

  async function importZip() {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: "存档压缩包", extensions: ["zip"] }],
      title: "选择世界 .zip",
    });
    if (typeof picked !== "string") return;
    setImporting(true);
    try {
      const folder = await api.importWorldZip(activeRoot(), props.instance.id, picked);
      toast({ type: "success", message: `已导入存档 ${folder}` });
      refetch();
    } catch (e) {
      toast({ type: "error", message: `导入失败:${e}` });
    } finally {
      setImporting(false);
    }
  }

  async function remove(w: WorldInfo) {
    try {
      await api.deleteWorld(activeRoot(), props.instance.id, w.folder);
      toast({ type: "success", message: `已删除存档 ${w.name}` });
      refetch();
    } catch (e) {
      toast({ type: "error", message: `删除失败:${e}` });
    }
  }

  async function backup(w: WorldInfo) {
    // 另存为:用户自定文件名/位置;同名文件由系统对话框确认覆盖,不会静默盖掉上次备份。
    const dest = await saveDialog({
      title: "备份存档为…",
      defaultPath: `${(w.name || w.folder).replace(/[\\/:*?"<>|]/g, "_")}-backup.zip`,
      filters: [{ name: "Zip 备份", extensions: ["zip"] }],
    });
    if (!dest) return; // 取消
    setBusy(w.folder);
    try {
      const zip = await api.backupWorld(activeRoot(), props.instance.id, w.folder, dest);
      toast({ type: "success", message: `已备份到 ${zip}` });
    } catch (e) {
      toast({ type: "error", message: `备份失败:${e}` });
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
      toast({ type: "success", message: `已重命名为 ${name}` });
      setEditing(null);
      refetch();
    } catch (e) {
      toast({ type: "error", message: `重命名失败:${e}` });
    }
  }

  const MODE_LABEL: Record<string, string> = {
    survival: "生存",
    creative: "创造",
    adventure: "冒险",
    spectator: "旁观",
    unknown: "未知",
  };

  return (
    <div class="flex flex-col gap-[8px]">
      <div class="flex items-center justify-between">
        <div class={LABEL}>世界列表(也可把 .zip 拖入此弹窗导入)</div>
        <div class="flex items-center gap-[4px]">
          <button
            class={OPEN_BTN}
            onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "saves")}
          >
            打开目录
          </button>
          <button
            class="text-[12px] text-a-6 px-[8px] py-[3px] rounded-xs cursor-pointer hover:bg-a-4/10 disabled:opacity-50"
            disabled={importing()}
            onClick={importZip}
          >
            {importing() ? "导入中…" : "导入存档…"}
          </button>
        </div>
      </div>

      <Show
        when={!worlds.loading}
        fallback={
          <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
            <Spinner size={16} /> 扫描存档…
          </div>
        }
      >
      <Show
        when={(worlds() ?? []).length > 0}
        fallback={<div class="text-dim text-[13px] py-[12px]">该实例还没有存档。</div>}
      >
        <div class="flex flex-col gap-[6px]">
          <For each={worlds()}>
            {(w) => (
              <div class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-ctl bg-n-3">
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
                  <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                    {[
                      MODE_LABEL[w.game_mode] ?? w.game_mode,
                      fmtSize(w.size_bytes),
                      w.seed != null ? `种子 ${w.seed}` : null,
                      w.folder,
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </div>
                </div>
                <button
                  class="shrink-0 text-[12px] text-dim px-[8px] py-[4px] rounded-xs cursor-pointer hover:text-fg hover:bg-a-4/10 disabled:opacity-50"
                  disabled={busy() === w.folder}
                  onClick={() => backup(w)}
                >
                  {busy() === w.folder ? "备份中…" : "备份"}
                </button>
                <button
                  class="shrink-0 text-[12px] text-dim px-[8px] py-[4px] rounded-xs cursor-pointer hover:text-fg hover:bg-a-4/10"
                  onClick={() => startRename(w)}
                >
                  重命名
                </button>
                <button class={DEL_BTN} onClick={() => remove(w)}>
                  删除
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>
      </Show>
    </div>
  );
};

export const InstanceManageDialog: Component<{
  open: boolean;
  instance: InstanceSummary | null;
  onClose: () => void;
  onChanged?: () => void;
  /** 复制完成回调,带新实例 id;ClassicLaunch 据此重拉列表并选中新实例。 */
  onCopied?: (newId: string) => void;
  /** 内嵌模式:不套 Dialog,直接铺在父容器里(Classic 右栏的「设置」标签用),
   *  隐藏实例名头部与「完成」按钮,父组件只在需要时挂载本组件即等于「打开」。 */
  embedded?: boolean;
  /** 受控 tab:启动页把实例管理页签提升到实例头部时使用。 */
  tab?: InstanceManageTab;
  onTabChange?: (tab: InstanceManageTab) => void;
  /** 隐藏本组件自带 tab 条,由外层渲染同级导航。 */
  hideTabs?: boolean;
}> = (props) => {
  const [internalTab, setInternalTab] = createSignal<InstanceManageTab>("settings");
  const [cfg, setCfg] = createSignal<InstanceConfig | null>(null);
  const [copying, setCopying] = createSignal(false);

  const tab = () => props.tab ?? internalTab();
  const setTab = (next: InstanceManageTab) => {
    setInternalTab(next);
    props.onTabChange?.(next);
  };
  const packKind = (): PackKind => {
    const t = tab();
    return isPackTab(t) ? t : "resource_pack";
  };

  // 是否「活动」(应加载数据 / 接受拖放):弹窗模式看 open,内嵌模式只要挂载即活动。
  const active = () => props.embedded || props.open;

  async function copyInstance() {
    const inst = props.instance;
    if (!inst) return;
    setCopying(true);
    try {
      const newId = await api.copyInstance(activeRoot(), inst.id, `${inst.name || inst.id} 副本`);
      toast({ type: "success", message: "已复制实例" });
      props.onCopied?.(newId);
    } catch (e) {
      toast({ type: "error", message: `复制失败:${e}` });
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
        .then(setCfg)
        .catch((e) => toast({ type: "error", message: `读取配置失败:${e}` }));
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
  const [query, setQuery] = createSignal("");
  const [debounced, setDebounced] = createSignal("");
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [modDetail, setModDetail] = createSignal<{ id: string; title: string; icon?: string | null } | null>(null);
  let debounceTimer: number | undefined;

  function onQueryInput(v: string) {
    setQuery(v);
    clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(() => setDebounced(v.trim()), 280);
  }
  onCleanup(() => clearTimeout(debounceTimer));

  // 搜索结果:仅在 Mods 标签 + 有关键词时请求。
  const [hits] = createResource(
    () =>
      active() && props.instance && tab() === "mods" && debounced()
        ? ([props.instance.id, debounced()] as const)
        : false,
    () => api.modrinthSearch(debounced(), "mod", props.instance?.mc_version ?? null, searchLoader()),
  );

  async function installHit(projectId: string, title: string) {
    const inst = props.instance;
    if (!inst) return;
    setInstalling(projectId);
    try {
      const report = await api.installMod(
        activeRoot(),
        inst.id,
        projectId,
        inst.mc_version,
        searchLoader() ?? "",
      );
      if (report.installed.length === 0 && report.unresolved.length === 0) {
        toast({ type: "info", message: `${title} 已存在,无需重复安装` });
      } else {
        const parts = [`已装入 ${report.installed.length} 个文件`];
        if (report.unresolved.length > 0)
          parts.push(`${report.unresolved.length} 个依赖未解决`);
        toast({
          type: report.unresolved.length > 0 ? "warn" : "success",
          message: `${title}:${parts.join(",")}`,
        });
      }
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
    } finally {
      setInstalling(null);
    }
  }

  // ---- Mod 更新检查 ----
  const [updates, setUpdates] = createSignal<ModUpdate[] | null>(null);
  const [checking, setChecking] = createSignal(false);
  const [updatingFile, setUpdatingFile] = createSignal<string | null>(null);

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
        message: list.length > 0 ? `发现 ${list.length} 个可更新` : "全部 mod 已是最新",
      });
    } catch (e) {
      toast({ type: "error", message: `检查更新失败:${e}` });
    } finally {
      setChecking(false);
    }
  }

  async function applyUpdate(u: ModUpdate) {
    const inst = props.instance;
    if (!inst) return;
    setUpdatingFile(u.file_name);
    try {
      await api.applyModUpdate(activeRoot(), inst.id, u);
      toast({ type: "success", message: `${u.name} 已更新到 ${u.new_version}` });
      setUpdates((prev) => (prev ?? []).filter((x) => x.file_name !== u.file_name));
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: `更新失败:${e}` });
    } finally {
      setUpdatingFile(null);
    }
  }

  async function applyAllUpdates() {
    for (const u of updates() ?? []) {
      // 串行执行,避免并发下载相互踩;每个失败只提示该项,不中断其余。
      await applyUpdate(u);
    }
  }

  // ---- 拖拽导入 ----
  // Tauri 启用了原生文件拖放,HTML5 ondrop 不触发,改用 webview 的 onDragDropEvent。
  // 整个弹窗作为拖放区,目标类型由当前标签决定。
  const [dragOver, setDragOver] = createSignal(false);
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
      toast({ type: "info", message: "请切到 Mods / 资源包 / 光影 / 数据包 / 存档标签再拖入文件" });
      return;
    }
    let ok = 0;
    for (const path of paths) {
      try {
        if (tab() === "worlds") await api.importWorldZip(activeRoot(), inst.id, path);
        else await api.importLocalResource(activeRoot(), inst.id, resourceTarget()!, path);
        ok += 1;
      } catch (e) {
        toast({ type: "error", message: `导入失败:${e}` });
      }
    }
    if (ok > 0) {
      toast({ type: "success", message: `已导入 ${ok} 个文件` });
      if (tab() === "mods") refetchMods();
      else if (tab() === "worlds") setWorldTick((t) => t + 1);
      else setImportTick((t) => t + 1);
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
      .catch((e) => toast({ type: "error", message: `保存失败:${e}` }));
  }

  async function pickIcon() {
    const inst = props.instance;
    if (!inst) return;
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: "图片", extensions: ["png", "jpg", "jpeg", "gif", "bmp", "webp"] }],
    });
    if (typeof picked !== "string") return; // 取消 / 多选(不会发生)
    try {
      await api.setInstanceIcon(activeRoot(), inst.id, picked);
      toast({ type: "success", message: "已更新实例图标" });
      props.onChanged?.(); // 触发列表重拉,新图标随 list_instances 探测回来
    } catch (e) {
      toast({ type: "error", message: `设置图标失败:${e}` });
    }
  }

  async function toggleMod(m: ModInfo, enabled: boolean) {
    const inst = props.instance;
    if (!inst) return;
    try {
      await api.setModEnabled(activeRoot(), inst.id, m.file_name, enabled);
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: `操作失败:${e}` });
    }
  }

  async function removeMod(m: ModInfo) {
    const inst = props.instance;
    if (!inst) return;
    try {
      await api.deleteMod(activeRoot(), inst.id, m.file_name);
      toast({ type: "success", message: `已删除 ${m.name}` });
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: `删除失败:${e}` });
    }
  }

  const body = (
      <div
        class="relative flex flex-col transition-shadow duration-150"
        classList={{
          "max-h-[calc(100vh-100px)]": !props.embedded,
          "h-full": props.embedded,
          "ring-2 ring-inset ring-a-4": dragOver(),
        }}
      >
        <Show when={dragOver() && dropAccepted()}>
          <div class="absolute inset-0 z-10 grid place-items-center bg-card/85 pointer-events-none">
            <div class="text-[14px] text-a-6 font-semibold">松手导入到此实例</div>
          </div>
        </Show>
        <Show when={!props.embedded}>
          <div class="px-[20px] pt-[18px] text-[15px] font-bold text-fg">
            {props.instance?.name || props.instance?.id}
          </div>
        </Show>

        <Show when={!props.hideTabs}>
          <div class="flex gap-[4px] px-[16px] border-b border-n-3 mt-[10px] overflow-x-auto">
            <For each={TABS}>
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

        <div class="p-[20px] flex flex-col gap-[14px] overflow-y-auto">
          {/* ---- 设置 ---- */}
          <Show when={tab() === "settings"}>
            <Show
              when={cfg()}
              fallback={
                <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
                  <Spinner size={16} /> 读取配置中…
                </div>
              }
            >
              {(c) => (
                <>
                  <div class="flex items-center gap-[12px]">
                    <div class="w-[56px] h-[56px] rounded-ctl overflow-hidden bg-n-3 shrink-0 grid place-items-center">
                      <Show
                        when={props.instance?.icon}
                        fallback={
                          <span class="text-[22px] font-bold text-dim">
                            {(props.instance?.name || props.instance?.id || "?").charAt(0).toUpperCase()}
                          </span>
                        }
                      >
                        <img src={props.instance!.icon!} alt="" width="56" height="56" class="w-full h-full object-cover" />
                      </Show>
                    </div>
                    <div class="flex flex-col gap-[5px]">
                      <span class={LABEL}>实例图标</span>
                      <button
                        class="h-[30px] px-[12px] border border-n-6 rounded-ctl bg-n-4 text-fg text-[12px] cursor-pointer transition-colors duration-150 hover:bg-n-5 w-fit"
                        onClick={pickIcon}
                      >
                        更换图标…
                      </button>
                    </div>
                  </div>

                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>名称</span>
                    <input
                      class={FIELD}
                      value={c().name ?? ""}
                      onChange={(e) => patch({ name: e.currentTarget.value || null })}
                    />
                  </label>

                  <div class="flex flex-col gap-[5px]">
                    <span class={LABEL}>最大内存 {c().memory_mb} MiB</span>
                    <input
                      class="w-full accent-[var(--a-4)]"
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
                    <span class={LABEL}>Java 路径(留空 = 跟随全局/自动)</span>
                    <input
                      class={FIELD}
                      placeholder="自动 / 全局设置"
                      value={c().java_path ?? ""}
                      onChange={(e) => patch({ java_path: e.currentTarget.value || null })}
                    />
                  </label>

                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>额外 JVM 参数(空格分隔)</span>
                    <input
                      class={FIELD}
                      value={c().jvm_args.join(" ")}
                      onChange={(e) =>
                        patch({ jvm_args: e.currentTarget.value.split(/\s+/).filter(Boolean) })
                      }
                    />
                  </label>

                  <div class="flex gap-[12px]">
                    <label class="flex-1 flex flex-col gap-[5px]">
                      <span class={LABEL}>窗口宽</span>
                      <input
                        class={FIELD}
                        type="number"
                        placeholder="默认"
                        value={c().width ?? ""}
                        onChange={(e) =>
                          patch({ width: e.currentTarget.value ? +e.currentTarget.value : null })
                        }
                      />
                    </label>
                    <label class="flex-1 flex flex-col gap-[5px]">
                      <span class={LABEL}>窗口高</span>
                      <input
                        class={FIELD}
                        type="number"
                        placeholder="默认"
                        value={c().height ?? ""}
                        onChange={(e) =>
                          patch({ height: e.currentTarget.value ? +e.currentTarget.value : null })
                        }
                      />
                    </label>
                  </div>

                  <label class="flex items-center justify-between text-fg text-[13px]">
                    <span>全屏启动</span>
                    <input
                      type="checkbox"
                      class="w-[16px] h-[16px] accent-[var(--a-4)] cursor-pointer"
                      checked={c().fullscreen}
                      onChange={(e) => patch({ fullscreen: e.currentTarget.checked })}
                    />
                  </label>

                  <div class="pt-[4px]">
                    <button
                      class="h-[30px] px-[12px] border border-n-6 rounded-ctl bg-n-4 text-fg text-[12px] cursor-pointer transition-colors duration-150 hover:bg-n-5"
                      onClick={() => props.instance && openInstanceDir(activeRoot(), props.instance.id)}
                    >
                      打开游戏目录
                    </button>
                  </div>
                </>
              )}
            </Show>
          </Show>

          {/* ---- Mods ---- */}
          <Show when={tab() === "mods"}>
            {/* 从 Modrinth 搜索并安装(按本实例的 MC 版本 + 加载器过滤) */}
            <div class="flex flex-col gap-[8px]">
              <div class="relative">
                <input
                  class={`${FIELD} w-full pr-[30px]`}
                  placeholder={`搜索 Modrinth mod(${props.instance?.mc_version ?? ""} · ${searchLoader() ?? "无加载器"})`}
                  value={query()}
                  onInput={(e) => onQueryInput(e.currentTarget.value)}
                />
                <Show when={hits.loading}>
                  <div class="absolute right-[10px] top-1/2 -translate-y-1/2">
                    <Spinner size={14} />
                  </div>
                </Show>
              </div>

              <Show when={searchLoader() === null}>
                <div class="text-[11px] text-dim">
                  该实例没有加载器(原版),无法安装 mod。
                </div>
              </Show>

              <Show when={debounced() && !hits.loading && (hits() ?? []).length === 0}>
                <div class="text-[12px] text-dim py-[4px]">没有匹配的 mod。</div>
              </Show>

              <Show when={(hits() ?? []).length > 0}>
                <div class="flex flex-col gap-[6px] max-h-[200px] overflow-y-auto">
                  <For each={hits()}>
                    {(h) => (
                      <div class="flex items-center gap-[10px] py-[7px] px-[10px] rounded-ctl bg-n-2 border border-n-3">
                        <Show
                          when={h.icon_url}
                          fallback={<div class="w-[30px] h-[30px] rounded-xs bg-n-4 shrink-0" />}
                        >
                          <img
                            src={h.icon_url!}
                            alt=""
                            width="30"
                            height="30"
                            class="w-[30px] h-[30px] rounded-xs object-cover shrink-0"
                          />
                        </Show>
                        <div class="flex-1 min-w-0">
                          <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                            {h.title}
                          </div>
                          <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                            {h.description}
                          </div>
                        </div>
                        <button
                          class={OPEN_BTN}
                          onClick={() => setModDetail({ id: h.project_id, title: h.title, icon: h.icon_url })}
                        >
                          详情
                        </button>
                        <button
                          class="shrink-0 h-[28px] px-[12px] rounded-ctl bg-a-4 text-white text-[12px] font-semibold cursor-pointer transition-opacity duration-150 hover:opacity-90 disabled:opacity-50 disabled:cursor-default"
                          disabled={installing() !== null}
                          onClick={() => installHit(h.project_id, h.title)}
                        >
                          {installing() === h.project_id ? "安装中…" : "安装"}
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>

              <div class="h-px bg-n-3 my-[2px]" />
              <div class="flex items-center justify-between">
                <div class={LABEL}>已安装</div>
                <div class="flex items-center gap-[4px]">
                  <button
                    class={OPEN_BTN}
                    onClick={() => openInstanceSubdir(activeRoot(), props.instance!.id, "mods")}
                  >
                    打开目录
                  </button>
                  <button
                    class="text-[12px] text-a-6 px-[8px] py-[3px] rounded-xs cursor-pointer hover:bg-a-4/10 disabled:opacity-50"
                    disabled={checking() || searchLoader() === null}
                    onClick={checkUpdates}
                  >
                    {checking() ? "检查中…" : "检查更新"}
                  </button>
                </div>
              </div>

              {/* 可更新清单(检查后才出现) */}
              <Show when={(updates() ?? []).length > 0}>
                <div class="flex flex-col gap-[6px] rounded-ctl bg-a-4/10 p-[8px]">
                  <div class="flex items-center justify-between">
                    <span class="text-[12px] text-fg font-semibold">
                      {updates()!.length} 个可更新
                    </span>
                    <button
                      class={INSTALL_BTN}
                      disabled={updatingFile() !== null}
                      onClick={applyAllUpdates}
                    >
                      全部更新
                    </button>
                  </div>
                  <For each={updates()}>
                    {(u) => (
                      <div class="flex items-center gap-[10px] py-[6px] px-[8px] rounded-ctl bg-card">
                        <div class="flex-1 min-w-0">
                          <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                            {u.name}
                          </div>
                          <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                            {(u.current_version ?? "当前") + " → " + u.new_version}
                          </div>
                        </div>
                        <button
                          class={INSTALL_BTN}
                          disabled={updatingFile() !== null}
                          onClick={() => applyUpdate(u)}
                        >
                          {updatingFile() === u.file_name ? "更新中…" : "更新"}
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </div>

            <Show
              when={!mods.loading}
              fallback={
                <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
                  <Spinner size={16} /> 扫描 mods…
                </div>
              }
            >
              <Show
                when={(mods() ?? []).length > 0}
                fallback={<div class="text-dim text-[13px] py-[12px]">该实例还没有 mod。</div>}
              >
                <div class="flex flex-col gap-[6px]">
                  <For each={mods()}>
                    {(m) => (
                      <div
                        class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-ctl bg-n-3"
                        classList={{ "opacity-55": !m.enabled }}
                      >
                        <div class="flex-1 min-w-0">
                          <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                            {m.name}
                          </div>
                          <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                            {[m.version, m.loader, m.file_name].filter(Boolean).join(" · ")}
                          </div>
                        </div>
                        <label class="flex items-center gap-[5px] text-[11px] text-dim cursor-pointer shrink-0">
                          <input
                            type="checkbox"
                            class="w-[15px] h-[15px] accent-[var(--a-4)] cursor-pointer"
                            checked={m.enabled}
                            onChange={(e) => toggleMod(m, e.currentTarget.checked)}
                          />
                          启用
                        </label>
                        <button
                          class="shrink-0 text-[12px] text-[#e5848a] px-[8px] py-[4px] rounded-xs cursor-pointer hover:bg-[rgba(229,132,138,0.14)]"
                          onClick={() => removeMod(m)}
                        >
                          删除
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
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
                    emptyHint="该实例还没有资源包。"
                    tick={importTick()}
                  />
                </Show>
                <Show when={packKind() === "shader"}>
                  <PacksPanel
                    instance={inst()}
                    kind="shader"
                    searchKind="shader"
                    emptyHint="该实例还没有光影。"
                    tick={importTick()}
                  />
                </Show>
                <Show when={packKind() === "datapack"}>
                  <PacksPanel
                    instance={inst()}
                    kind="datapack"
                    searchKind="datapack"
                    emptyHint="该实例还没有数据包。"
                    tick={importTick()}
                  />
                </Show>
              </>
            )}
          </Show>

          {/* ---- 存档 ---- */}
          <Show when={tab() === "worlds" && props.instance}>
            {(inst) => <WorldsPanel instance={inst()} tick={worldTick()} />}
          </Show>

          {/* ---- 截图 ---- */}
          <Show when={tab() === "screenshots" && props.instance}>
            {(inst) => <ScreenshotsPanel instance={inst()} />}
          </Show>

          {/* Mods 搜索结果的项目详情(覆盖整个弹窗) */}
          <Show when={modDetail() && props.instance}>
            <ProjectDetailPanel
              projectId={modDetail()!.id}
              title={modDetail()!.title}
              iconUrl={modDetail()!.icon}
              target="mod"
              instanceId={props.instance!.id}
              mcVersion={props.instance!.mc_version}
              loader={searchLoader()}
              onClose={() => setModDetail(null)}
              onInstalled={() => refetchMods()}
            />
          </Show>
        </div>

        <div class="flex justify-between items-center px-[20px] py-[14px] border-t border-n-3">
          <button
            class="h-[34px] px-[16px] border border-n-6 rounded-ctl bg-transparent text-dim text-[13px] cursor-pointer transition-colors duration-150 hover:text-fg hover:bg-n-4 disabled:opacity-50"
            disabled={copying() || !props.instance}
            onClick={copyInstance}
          >
            {copying() ? "复制中…" : "复制实例"}
          </button>
          <Show when={!props.embedded}>
            <button
              class="h-[34px] px-[16px] border border-n-6 rounded-ctl bg-n-4 text-fg text-[13px] cursor-pointer transition-colors duration-150 hover:bg-n-5"
              onClick={props.onClose}
            >
              完成
            </button>
          </Show>
        </div>
      </div>
  );

  // 内嵌模式直接铺在父容器;否则套 Dialog 作模态。
  return props.embedded ? (
    body
  ) : (
    <Dialog
      open={props.open}
      onClose={props.onClose}
      label="实例管理"
      contentClass="w-[520px] max-w-[calc(100vw-48px)] bg-card rounded-card shadow-card overflow-hidden"
    >
      {body}
    </Dialog>
  );
};

export default InstanceManageDialog;
