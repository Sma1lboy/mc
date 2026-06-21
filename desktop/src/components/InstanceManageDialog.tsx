import { Component, createSignal, createResource, createEffect, onCleanup, For, Show } from "solid-js";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Dialog } from "./Dialog";
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { activeRoot } from "../store";
import type {
  InstanceConfig,
  InstanceSummary,
  ModInfo,
  PackKind,
  PackInfo,
  ProjectKind,
  WorldInfo,
} from "../ipc/types";

/**
 * InstanceManageDialog —— 单实例管理:设置(名字/内存/Java/JVM/窗口)+ Mods(启停/删除)。
 * 设置改一项即 set_instance_config 持久化;Mods 用 set_mod_enabled / delete_mod。
 */

const FIELD =
  "h-[34px] px-[12px] rounded-ctl border border-n-6 bg-n-2 text-fg text-[13px] outline-none " +
  "transition-colors duration-150 focus:border-a-4";
const LABEL = "text-[12px] text-dim";
const TAB =
  "px-[14px] py-[7px] text-[13px] font-semibold cursor-pointer border-b-2 border-b-transparent " +
  "text-n-6 hover:text-n-8 transition-colors duration-150";
const TAB_ACTIVE = "!text-a-6 !border-b-a-5";

type Tab = "settings" | "mods" | "resources" | "worlds";

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
}> = (props) => {
  const [packs, { refetch }] = createResource(
    () => [props.instance.id, props.kind] as const,
    ([id, kind]) => api.instancePacks(activeRoot(), id, kind),
  );

  const [query, setQuery] = createSignal("");
  const [debounced, setDebounced] = createSignal("");
  const [installing, setInstalling] = createSignal<string | null>(null);
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
      await api.setPackEnabled(activeRoot(), props.instance.id, props.kind, p.file_name, enabled);
      refetch();
    } catch (e) {
      toast({ type: "error", message: `操作失败:${e}` });
    }
  }

  async function remove(p: PackInfo) {
    try {
      await api.deletePack(activeRoot(), props.instance.id, props.kind, p.file_name);
      toast({ type: "success", message: `已删除 ${p.file_name}` });
      refetch();
    } catch (e) {
      toast({ type: "error", message: `删除失败:${e}` });
    }
  }

  return (
    <div class="flex flex-col gap-[8px]">
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
                  <img src={h.icon_url!} alt="" class="w-[30px] h-[30px] rounded-xs object-cover shrink-0" />
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
                  class={INSTALL_BTN}
                  disabled={installing() !== null}
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
      <div class={LABEL}>已安装</div>

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
    </div>
  );
};

/**
 * WorldsPanel —— 存档世界列表 + 备份(导出 zip)/ 重命名(改显示名)/ 删除(走回收站)。
 */
const WorldsPanel: Component<{ instance: InstanceSummary }> = (props) => {
  const [worlds, { refetch }] = createResource(
    () => props.instance.id,
    (id) => api.instanceWorlds(activeRoot(), id),
  );

  // 行内重命名:正在编辑的世界 folder + 草稿名。
  const [editing, setEditing] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [busy, setBusy] = createSignal<string | null>(null);

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
    const dir = await openDialog({ directory: true, title: "选择备份保存位置" });
    if (typeof dir !== "string") return; // 取消
    setBusy(w.folder);
    try {
      const zip = await api.backupWorld(activeRoot(), props.instance.id, w.folder, dir);
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
                      autofocus
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
                    {[MODE_LABEL[w.game_mode] ?? w.game_mode, fmtSize(w.size_bytes), w.folder]
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
  );
};

export const InstanceManageDialog: Component<{
  open: boolean;
  instance: InstanceSummary | null;
  onClose: () => void;
  onChanged?: () => void;
}> = (props) => {
  const [tab, setTab] = createSignal<Tab>("settings");
  const [cfg, setCfg] = createSignal<InstanceConfig | null>(null);
  // 资源标签内的子类型:资源包 / 光影 / 数据包。
  const [resKind, setResKind] = createSignal<PackKind>("resource_pack");

  // 打开/切换实例时拉配置 + 复位到设置页;关闭时清空。
  createEffect(() => {
    const inst = props.instance;
    if (props.open && inst) {
      setCfg(null);
      api
        .getInstanceConfig(activeRoot(), inst.id)
        .then(setCfg)
        .catch((e) => toast({ type: "error", message: `读取配置失败:${e}` }));
    } else if (!props.open) {
      setCfg(null);
      setTab("settings");
    }
  });

  // Mods:仅在 Mods 标签 + 弹窗打开时拉取。
  const [mods, { refetch: refetchMods }] = createResource(
    () => (props.open && props.instance && tab() === "mods" ? props.instance.id : false),
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
      props.open && props.instance && tab() === "mods" && debounced()
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

  return (
    <Dialog
      open={props.open}
      onClose={props.onClose}
      label="实例管理"
      contentClass="w-[520px] max-w-[calc(100vw-48px)] bg-card rounded-card shadow-card overflow-hidden focus:outline-none"
    >
      <div class="flex flex-col max-h-[calc(100vh-100px)]">
        <div class="px-[20px] pt-[18px] text-[15px] font-bold text-fg">
          {props.instance?.name || props.instance?.id}
        </div>

        <div class="flex gap-[4px] px-[16px] border-b border-n-3 mt-[10px]">
          <button class={`${TAB} ${tab() === "settings" ? TAB_ACTIVE : ""}`} onClick={() => setTab("settings")}>
            设置
          </button>
          <button class={`${TAB} ${tab() === "mods" ? TAB_ACTIVE : ""}`} onClick={() => setTab("mods")}>
            Mods
          </button>
          <button class={`${TAB} ${tab() === "resources" ? TAB_ACTIVE : ""}`} onClick={() => setTab("resources")}>
            资源
          </button>
          <button class={`${TAB} ${tab() === "worlds" ? TAB_ACTIVE : ""}`} onClick={() => setTab("worlds")}>
            存档
          </button>
        </div>

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
                        <img src={props.instance!.icon!} alt="" class="w-full h-full object-cover" />
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
              <div class={LABEL}>已安装</div>
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

          {/* ---- 资源(资源包 / 光影)---- */}
          <Show when={tab() === "resources" && props.instance}>
            {(inst) => (
              <>
                <div class="flex gap-[6px]">
                  <button
                    class={`px-[12px] h-[28px] rounded-ctl text-[12px] cursor-pointer transition-colors duration-150 ${
                      resKind() === "resource_pack" ? "bg-a-4 text-white" : "bg-n-3 text-dim hover:text-fg"
                    }`}
                    onClick={() => setResKind("resource_pack")}
                  >
                    资源包
                  </button>
                  <button
                    class={`px-[12px] h-[28px] rounded-ctl text-[12px] cursor-pointer transition-colors duration-150 ${
                      resKind() === "shader" ? "bg-a-4 text-white" : "bg-n-3 text-dim hover:text-fg"
                    }`}
                    onClick={() => setResKind("shader")}
                  >
                    光影
                  </button>
                  <button
                    class={`px-[12px] h-[28px] rounded-ctl text-[12px] cursor-pointer transition-colors duration-150 ${
                      resKind() === "datapack" ? "bg-a-4 text-white" : "bg-n-3 text-dim hover:text-fg"
                    }`}
                    onClick={() => setResKind("datapack")}
                  >
                    数据包
                  </button>
                </div>
                <Show when={resKind() === "resource_pack"}>
                  <PacksPanel
                    instance={inst()}
                    kind="resource_pack"
                    searchKind="resourcepack"
                    emptyHint="该实例还没有资源包。"
                  />
                </Show>
                <Show when={resKind() === "shader"}>
                  <PacksPanel
                    instance={inst()}
                    kind="shader"
                    searchKind="shader"
                    emptyHint="该实例还没有光影。"
                  />
                </Show>
                <Show when={resKind() === "datapack"}>
                  <PacksPanel
                    instance={inst()}
                    kind="datapack"
                    searchKind="datapack"
                    emptyHint="该实例还没有数据包。"
                  />
                </Show>
              </>
            )}
          </Show>

          {/* ---- 存档 ---- */}
          <Show when={tab() === "worlds" && props.instance}>
            {(inst) => <WorldsPanel instance={inst()} />}
          </Show>
        </div>

        <div class="flex justify-end px-[20px] py-[14px] border-t border-n-3">
          <button
            class="h-[34px] px-[16px] border border-n-6 rounded-ctl bg-n-4 text-fg text-[13px] cursor-pointer transition-colors duration-150 hover:bg-n-5"
            onClick={props.onClose}
          >
            完成
          </button>
        </div>
      </div>
    </Dialog>
  );
};

export default InstanceManageDialog;
