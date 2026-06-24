import { Component, createResource, createSignal, For, Show } from "solid-js";
import { api } from "../ipc/api";
import { activeRoot } from "../store";
import { toast } from "./Toast";
import { Spinner } from "./Spinner";
import { Panel } from "./Panel";
import { Heading } from "./Typography";
import { Toggle } from "./Toggle";
import { ACCENT_BTN_COMPACT } from "./styles";
import { renderMarkdown } from "../util/markdown";
import { acceptedLoaders } from "../util/loaders";
import { t } from "../i18n";
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
  /** 数据包安装的目标存档(逐存档生效);其它类型传 null */
  world?: string | null;
  /** 内容来源平台(modrinth / curseforge);决定走哪个 provider。缺省 modrinth。 */
  provider?: string;
  onClose: () => void;
  onInstalled: () => void;
}> = (props) => {
  const [project] = createResource(
    () => props.projectId,
    (id) => api.modrinthProject(id),
  );
  const [versions] = createResource(
    () => props.projectId,
    (id) => api.modrinthVersions(id, props.provider ?? "modrinth"),
  );
  const [installing, setInstalling] = createSignal<string | null>(null);
  const [showAll, setShowAll] = createSignal(false);

  function compatible(v: ModrinthVersion): boolean {
    const okGame = v.game_versions.includes(props.mcVersion);
    // loader 为 null(资源包/光影)不按加载器分;Quilt 实例同时接受 fabric 版本。
    const okLoader =
      props.loader == null || acceptedLoaders(props.loader).some((l) => v.loaders.includes(l));
    return okGame && okLoader;
  }
  // 默认只显示兼容版本;没有兼容版本时回退显示全部(否则空列表会让人误以为没版本)。
  const shown = () => {
    const all = versions() ?? [];
    const compat = all.filter(compatible);
    return showAll() || compat.length === 0 ? all : compat;
  };

  // 仅 mod 的加载器/版本不匹配是硬性不可装(必崩);资源包/光影的版本差异只是软提示。
  const blocked = (v: ModrinthVersion) => props.target === "mod" && !compatible(v);

  async function install(v: ModrinthVersion) {
    if (blocked(v)) {
      toast({ type: "error", message: t("projectDetail.incompatibleVersion") });
      return;
    }
    setInstalling(v.id);
    try {
      // mod 传 mc/loader 以便一并解析 required 依赖;资源包/光影/数据包不需要。
      const isMod = props.target === "mod";
      const report = await api.installVersionFile(
        activeRoot(),
        props.instanceId,
        props.target,
        v.id,
        isMod ? props.mcVersion : null,
        isMod ? props.loader : null,
        props.target === "datapack" ? props.world ?? null : null,
        props.provider ?? "modrinth",
        props.projectId,
      );
      const parts = [t("projectDetail.installedVersion", { version: v.version_number })];
      if (report.installed_deps > 0) parts.push(t("projectDetail.depsAdded", { count: report.installed_deps }));
      if (report.unresolved.length > 0) parts.push(t("projectDetail.depsUnresolved", { count: report.unresolved.length }));
      const conflicts = report.incompatible?.length ?? 0;
      if (conflicts > 0) parts.push(t("projectDetail.declaredConflicts", { count: conflicts }));
      toast({ type: report.unresolved.length > 0 || conflicts > 0 ? "warn" : "success", message: parts.join(",") });
      props.onInstalled();
    } catch (e) {
      toast({ type: "error", message: t("projectDetail.installFailed", { error: String(e) }) });
    } finally {
      setInstalling(null);
    }
  }

  function fmtDate(iso: string): string {
    // 仅取日期部分,避免引入时区/本地化复杂度。
    return iso.slice(0, 10);
  }

  return (
    <Panel as="div" variant="sunken" class="absolute inset-0 z-20 flex flex-col bg-window">
      <div class="flex items-center gap-[10px] px-[16px] py-[12px] border-b border-titlebar">
        <button
          class="shrink-0 h-[28px] px-[12px] rounded-none bg-panel-3 text-fg text-[12px] cursor-pointer shadow-raised active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
          onClick={props.onClose}
        >
          {t("projectDetail.back")}
        </button>
        <Show when={props.iconUrl}>
          <img src={props.iconUrl!} alt="" width="26" height="26" class="w-[26px] h-[26px] rounded-none object-cover shrink-0 shadow-sunken" style="image-rendering:pixelated" />
        </Show>
        <Heading size="sub" class="whitespace-nowrap overflow-hidden text-ellipsis">
          {props.title}
        </Heading>
      </div>

      <div class="flex-1 overflow-y-auto p-[16px] flex flex-col gap-[16px]">
        {/* 简介 */}
        <Show
          when={!project.loading}
          fallback={
            <div class="flex items-center gap-[10px] text-muted text-[13px] py-[8px]">
              <Spinner size={16} /> {t("projectDetail.loadingAbout")}
            </div>
          }
        >
          <Show
            when={project()?.body?.trim()}
            fallback={<div class="text-muted text-[13px]">{t("projectDetail.noAbout")}</div>}
          >
            {/* renderMarkdown 转义优先,输出仅含白名单标签,innerHTML 安全 */}
            <div class="md text-[13px] text-sub" innerHTML={renderMarkdown(project()!.body)} />
          </Show>
        </Show>

        {/* 版本 */}
        <div class="flex items-center justify-between">
          <Heading size="sub">{t("projectDetail.versions")}</Heading>
          <label class="flex items-center gap-[8px] text-[11px] text-muted cursor-pointer select-none">
            {t("projectDetail.showAllVersions")}
            <Toggle checked={showAll()} onChange={setShowAll} />
          </label>
        </div>

        <Show
          when={!versions.loading}
          fallback={
            <div class="flex items-center gap-[10px] text-muted text-[13px] py-[8px]">
              <Spinner size={16} /> {t("projectDetail.loadingVersions")}
            </div>
          }
        >
          <Show
            when={shown().length > 0}
            fallback={<div class="text-muted text-[13px]">{t("projectDetail.noVersions")}</div>}
          >
            <div class="flex flex-col gap-[6px]">
              <For each={shown()}>
                {(v) => (
                  <Panel
                    variant="sunken"
                    class="flex items-center gap-[10px] py-[8px] px-[10px] bg-panel-2"
                    classList={{ "opacity-60": !compatible(v) }}
                  >
                    <div class="flex-1 min-w-0">
                      <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                        {v.version_number}
                        <span class="text-[11px] text-muted ml-[6px]">{v.version_type}</span>
                      </div>
                      <div class="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
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
                      class={ACCENT_BTN_COMPACT}
                      disabled={installing() !== null || blocked(v)}
                      title={blocked(v) ? t("projectDetail.incompatibleTooltip") : ""}
                      onClick={() => install(v)}
                    >
                      {installing() === v.id ? t("projectDetail.installing") : t("projectDetail.install")}
                    </button>
                  </Panel>
                )}
              </For>
            </div>
          </Show>
        </Show>
      </div>
    </Panel>
  );
};

export default ProjectDetailPanel;
