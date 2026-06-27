import { Component, createResource, createSignal, For, Show } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner } from "./Spinner";
import { ErrorState } from "./ErrorState";
import Lightbox from "./Lightbox";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { cached } from "../ipc/cache";
import { t } from "../i18n";
import { renderMarkdown } from "../util/markdown";
import type { LightboxImage } from "./Lightbox";
import "../pages/ModpackDetail.css"; // .md markdown 标记样式

/**
 * ModpackOverview —— 整合包来源实例「概览」标签:只读展示该整合包的 Modrinth 详情
 * (图标/简介/分类/链接/画廊/正文)。不含安装流程(实例已装),区别于 ModpackDetail。
 */
export const ModpackOverview: Component<{ projectId: string }> = (props) => {
  const [project, { refetch }] = createResource(
    () => props.projectId,
    (id) =>
      cached(`project|modrinth|${id}`, () => api.modrinthProject(id)).catch((e) => {
        toast({ type: "error", message: t("discover.modpackDetailLoadFailed", { error: String(e) }) });
        return null;
      }),
  );
  const [lb, setLb] = createSignal<number | null>(null);
  const gallery = (): LightboxImage[] => project()?.gallery ?? [];
  const links = () => {
    const p = project();
    if (!p) return [] as { label: string; url: string }[];
    return [
      { label: t("discover.linkSource"), url: p.source_url },
      { label: t("discover.linkIssues"), url: p.issues_url },
      { label: t("discover.linkWiki"), url: p.wiki_url },
      { label: t("discover.linkDiscord"), url: p.discord_url },
    ].filter((l) => !!l.url) as { label: string; url: string }[];
  };

  return (
    <Show
      when={!project.loading}
      fallback={
        <div class="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
          <Spinner size={16} /> {t("discover.loadingModpackDetail")}
        </div>
      }
    >
      <Show when={project()} fallback={<ErrorState message={t("discover.modpackDetailUnavailable")} onRetry={() => void refetch()} />}>
        {(p) => (
          <div class="flex flex-col gap-[16px]">
            {/* 品牌信息(图标/名称/下载量/分类)已上移到实例头部,此处不再重复;概览只留简介/链接/画廊/正文。 */}
            <Show when={p().description}>
              <p class="m-0 text-[13px] leading-[1.7] text-sub">{p().description}</p>
            </Show>

            <Show when={links().length}>
              <div class="flex flex-wrap gap-[6px]">
                <For each={links()}>
                  {(l) => (
                    <button
                      class="h-[28px] px-[12px] rounded-none bg-panel-3 text-tag text-[12px] cursor-pointer shadow-raised active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
                      onClick={() => shellOpen(l.url)}
                    >
                      {l.label} ↗
                    </button>
                  )}
                </For>
              </div>
            </Show>

            <Show when={gallery().length}>
              <div class="grid grid-cols-3 gap-[8px]">
                <For each={gallery()}>
                  {(g, i) => (
                    <img
                      class="w-full aspect-video object-cover rounded-none cursor-zoom-in bg-panel-2 shadow-sunken"
                      src={g.url}
                      alt={g.title ?? ""}
                      width="320"
                      height="180"
                      loading="lazy"
                      onClick={() => setLb(i())}
                    />
                  )}
                </For>
              </div>
            </Show>

            <Show when={p().body?.trim()}>
              <div class="md text-[13px] leading-[1.7] text-sub" innerHTML={renderMarkdown(p().body)} />
            </Show>

            <Show when={lb() !== null}>
              <Lightbox images={gallery()} index={lb()!} onIndex={setLb} onClose={() => setLb(null)} />
            </Show>
          </div>
        )}
      </Show>
    </Show>
  );
};

export default ModpackOverview;
