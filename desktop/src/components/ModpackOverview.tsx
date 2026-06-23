import { Component, createResource, createSignal, For, Show } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner } from "./Spinner";
import { ErrorState } from "./ErrorState";
import Lightbox from "./Lightbox";
import { toast } from "./Toast";
import { api } from "../ipc/api";
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
      api.modrinthProject(id).catch((e) => {
        toast({ type: "error", message: `整合包详情加载失败:${e}` });
        return null;
      }),
  );
  const [lb, setLb] = createSignal<number | null>(null);
  const gallery = (): LightboxImage[] => project()?.gallery ?? [];
  const links = () => {
    const p = project();
    if (!p) return [] as { label: string; url: string }[];
    return [
      { label: "源码", url: p.source_url },
      { label: "问题反馈", url: p.issues_url },
      { label: "Wiki", url: p.wiki_url },
      { label: "Discord", url: p.discord_url },
    ].filter((l) => !!l.url) as { label: string; url: string }[];
  };

  return (
    <Show
      when={!project.loading}
      fallback={
        <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
          <Spinner size={16} /> 加载整合包详情…
        </div>
      }
    >
      <Show when={project()} fallback={<ErrorState message="无法加载整合包详情" onRetry={() => void refetch()} />}>
        {(p) => (
          <div class="flex flex-col gap-[16px]">
            {/* 头部 */}
            <div class="flex gap-[14px] items-center">
              <Show
                when={p().icon_url}
                fallback={
                  <div class="w-[64px] h-[64px] rounded-[14px] grid place-items-center text-[28px] font-bold text-white bg-[linear-gradient(135deg,var(--a-3),var(--a-5))] shrink-0">
                    {(p().title[0] ?? "?").toUpperCase()}
                  </div>
                }
              >
                <img class="w-[64px] h-[64px] rounded-[14px] object-cover shrink-0" src={p().icon_url!} width="64" height="64" alt="" />
              </Show>
              <div class="min-w-0">
                <h2 class="m-0 text-[20px] font-extrabold text-fg whitespace-nowrap overflow-hidden text-ellipsis">{p().title}</h2>
                <div class="mt-[3px] text-[12px] text-dim">
                  ⬇ {p().downloads.toLocaleString()}
                  <Show when={p().followers}>{" · ♥ "}{p().followers.toLocaleString()}</Show>
                </div>
                <div class="mt-[6px] flex flex-wrap gap-[6px]">
                  <For each={p().categories}>
                    {(c) => <span class="text-[11px] py-[2px] px-[8px] rounded-full bg-a-1 text-a-6 capitalize">{c}</span>}
                  </For>
                </div>
              </div>
            </div>

            <Show when={p().description}>
              <p class="m-0 text-[13px] leading-[1.7] text-dim">{p().description}</p>
            </Show>

            <Show when={links().length}>
              <div class="flex flex-wrap gap-[6px]">
                <For each={links()}>
                  {(l) => (
                    <button
                      class="h-[28px] px-[12px] rounded-ctl border border-glass-border bg-glass-card text-a-6 text-[12px] cursor-pointer transition-colors duration-150 hover:bg-glass-hover"
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
                      class="w-full aspect-video object-cover rounded-ctl cursor-zoom-in bg-glass-card"
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
              <div class="md text-[13px] leading-[1.7] text-dim" innerHTML={renderMarkdown(p().body)} />
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
