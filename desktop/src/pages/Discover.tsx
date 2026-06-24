import { Component, createSignal, createEffect, For, Show } from "solid-js";
import { ContentBrowser, type ModpackHit } from "../components";
import type { ContentProvider } from "../components/ContentBrowser";
import type { ProjectKind } from "../ipc/types";
import { discoverTarget, setDiscoverTarget } from "../store";
import { t } from "../i18n";
import ModpackDetail from "./ModpackDetail";
import ProjectInstallDetail from "./ProjectInstallDetail";

/**
 * Discover —— Modrinth 搜索页。类型切换 + 防抖搜索 + 列表。
 * 搜索/分页/列表渲染复用 <ContentBrowser>;Discover 不绑定实例,故不按
 * 版本/加载器过滤(mcVersion="" + loader=null),点击行打开详情页而非直接下载。
 */

const KINDS = (): { key: ProjectKind; label: string }[] => [
  { key: "modpack", label: t("discover.kindModpack") },
  { key: "mod", label: t("discover.kindMod") },
  { key: "shader", label: t("discover.kindShader") },
  { key: "resourcepack", label: t("discover.kindResourcepack") },
  { key: "datapack", label: t("discover.kindDatapack") },
];

type SelectedProject = { hit: ModpackHit; kind: ProjectKind; provider: ContentProvider };

const Discover: Component = () => {
  const [kind, setKind] = createSignal<ProjectKind>("modpack");

  // 当前打开详情的项目(null = 显示搜索网格)。点击卡片/按钮进入详情页,而非直接下载。
  const [selected, setSelected] = createSignal<SelectedProject | null>(null);

  function openHit(h: ModpackHit, provider: ContentProvider) {
    setSelected({ hit: h, kind: kind(), provider });
  }

  // 从首页「发现」卡片跳进来时,自动打开目标项目详情(消费一次即清空)。
  createEffect(() => {
    const t = discoverTarget();
    if (!t) return;
    setKind(t.kind);
    setSelected({ hit: t.hit, kind: t.kind, provider: "modrinth" });
    setDiscoverTarget(null);
  });

  return (
    <div class="px-[28px] py-[24px] overflow-y-auto h-full">
      <Show when={selected()}>
        {(project) => (
          <Show
            when={project().kind === "modpack"}
            fallback={
              <ProjectInstallDetail
                hit={project().hit}
                kind={project().kind as Exclude<ProjectKind, "modpack">}
                provider={project().provider}
                onBack={() => setSelected(null)}
              />
            }
          >
            <ModpackDetail hit={project().hit} onBack={() => setSelected(null)} />
          </Show>
        )}
      </Show>

      <Show when={!selected()}>
      <div class="flex items-center justify-between gap-[16px] mb-[16px]">
        <h1 class="text-[24px] font-bold text-fg m-0">{t("discover.heading")}</h1>
      </div>

      <div class="flex gap-[8px] mb-[16px]">
        <For each={KINDS()}>
          {(k) => (
            <button
              class="px-[14px] py-[6px] border-none rounded-ctl text-[13px] cursor-pointer transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5 focus-visible:ring-offset-2 focus-visible:ring-offset-n-1"
              classList={{
                "bg-a-4 text-white": kind() === k.key,
                "bg-glass-card text-dim hover:bg-glass-hover hover:text-fg": kind() !== k.key,
              }}
              onClick={() => setKind(k.key)}
            >
              {k.label}
            </button>
          )}
        </For>
      </div>

      {/* 切类型时整体重挂 ContentBrowser,清空上一类型的搜索词/分页。
          Discover 不绑定实例:mcVersion="" + loader=null;点击行或「添加」均打开详情页。 */}
      <Show when={kind()} keyed>
        {(k) => (
          <ContentBrowser
            kind={k}
            mcVersion=""
            loader={null}
            onAdd={openHit}
            onOpenDetail={openHit}
            placeholder={t("discover.searchPlaceholder")}
          />
        )}
      </Show>
      </Show>
    </div>
  );
};

export default Discover;
