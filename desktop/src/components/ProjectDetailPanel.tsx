import { Component, createResource, createSignal, For, Show } from "solid-js";
import { api } from "../ipc/api";
import { activeRoot } from "../store";
import { toast } from "./Toast";
import { Spinner } from "./Spinner";
import { renderMarkdown } from "../util/markdown";
import type { ModrinthVersion } from "../ipc/types";
import "../pages/ModpackDetail.css"; // 复用 .md markdown 排版

/**
 * ProjectDetailPanel —— 在实例管理弹窗内,安装前先看 Modrinth 项目的简介与版本列表。
 *
 * 以绝对定位覆盖整个弹窗内容(最近的定位祖先是弹窗的 relative 容器),「返回」关闭。
 * 默认只列出与本实例 mc 版本 +(mod 的)加载器兼容的版本;可切到「全部版本」。
 * 安装走 install_version_file(指定 version id,不解析依赖 —— 用户已显式选版)。
 */
export const ProjectDetailPanel: Component<{
  projectId: string;
  title: string;
  iconUrl?: string | null;
  /** "mod" | "resourcepack" | "shader" | "datapack" */
  target: string;
  instanceId: string;
  mcVersion: string;
  /** mod 用加载器过滤;资源包/光影/数据包传 null(不按加载器分) */
  loader: string | null;
  onClose: () => void;
  onInstalled: () => void;
}> = (props) => {
  const [project] = createResource(
    () => props.projectId,
    (id) => api.modrinthProject(id),
  );
  const [versions] = createResource(
    () => props.projectId,
    (id) => api.modrinthVersions(id),
  );
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [showAll, setShowAll] = createSignal(false);

  function compatible(v: ModrinthVersion): boolean {
    const okGame = v.game_versions.includes(props.mcVersion);
    const okLoader = props.loader == null || v.loaders.includes(props.loader);
    return okGame && okLoader;
  }
  // 默认只显示兼容版本;没有兼容版本时回退显示全部(否则空列表会让人误以为没版本)。
  const shown = () => {
    const all = versions() ?? [];
    const compat = all.filter(compatible);
    return showAll() || compat.length === 0 ? all : compat;
  };

  async function install(v: ModrinthVersion) {
    setInstalling(v.id);
    try {
      const file = await api.installVersionFile(activeRoot(), props.instanceId, props.target, v.id);
      toast({ type: "success", message: `已安装 ${v.version_number}(${file})` });
      props.onInstalled();
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
    } finally {
      setInstalling(null);
    }
  }

  function fmtDate(iso: string): string {
    // 仅取日期部分,避免引入时区/本地化复杂度。
    return iso.slice(0, 10);
  }

  return (
    <div class="absolute inset-0 z-20 flex flex-col bg-card">
      <div class="flex items-center gap-[10px] px-[16px] py-[12px] border-b border-n-3">
        <button
          class="h-[28px] px-[10px] rounded-ctl border border-n-6 bg-n-4 text-fg text-[12px] cursor-pointer hover:bg-n-5"
          onClick={props.onClose}
        >
          ← 返回
        </button>
        <Show when={props.iconUrl}>
          <img src={props.iconUrl!} alt="" width="26" height="26" class="w-[26px] h-[26px] rounded-xs object-cover shrink-0" />
        </Show>
        <div class="text-[15px] font-bold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
          {props.title}
        </div>
      </div>

      <div class="flex-1 overflow-y-auto p-[16px] flex flex-col gap-[16px]">
        {/* 简介 */}
        <Show
          when={!project.loading}
          fallback={
            <div class="flex items-center gap-[10px] text-dim text-[13px] py-[8px]">
              <Spinner size={16} /> 加载简介…
            </div>
          }
        >
          <Show
            when={project()?.body?.trim()}
            fallback={<div class="text-dim text-[13px]">该项目没有简介。</div>}
          >
            {/* renderMarkdown 转义优先,输出仅含白名单标签,innerHTML 安全 */}
            <div class="md text-[13px] text-fg" innerHTML={renderMarkdown(project()!.body)} />
          </Show>
        </Show>

        {/* 版本 */}
        <div class="flex items-center justify-between">
          <div class="text-[12px] text-dim">版本</div>
          <label class="flex items-center gap-[5px] text-[11px] text-dim cursor-pointer">
            <input
              type="checkbox"
              class="w-[14px] h-[14px] accent-[var(--a-4)] cursor-pointer"
              checked={showAll()}
              onChange={(e) => setShowAll(e.currentTarget.checked)}
            />
            显示全部版本
          </label>
        </div>

        <Show
          when={!versions.loading}
          fallback={
            <div class="flex items-center gap-[10px] text-dim text-[13px] py-[8px]">
              <Spinner size={16} /> 加载版本…
            </div>
          }
        >
          <Show
            when={shown().length > 0}
            fallback={<div class="text-dim text-[13px]">没有可用版本。</div>}
          >
            <div class="flex flex-col gap-[6px]">
              <For each={shown()}>
                {(v) => (
                  <div
                    class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-ctl bg-n-2 border border-n-3"
                    classList={{ "opacity-60": !compatible(v) }}
                  >
                    <div class="flex-1 min-w-0">
                      <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                        {v.version_number}
                        <span class="text-[11px] text-dim ml-[6px]">{v.version_type}</span>
                      </div>
                      <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                        {[
                          v.game_versions.join(", "),
                          v.loaders.join(", "),
                          fmtDate(v.date_published),
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </div>
                    </div>
                    <button
                      class="shrink-0 h-[28px] px-[12px] rounded-ctl bg-a-4 text-white text-[12px] font-semibold cursor-pointer transition-opacity duration-150 hover:opacity-90 disabled:opacity-50 disabled:cursor-default"
                      disabled={installing() !== null}
                      onClick={() => install(v)}
                    >
                      {installing() === v.id ? "安装中…" : "安装"}
                    </button>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </Show>
      </div>
    </div>
  );
};

export default ProjectDetailPanel;
