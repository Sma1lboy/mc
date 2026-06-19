import { Component, createResource, createSignal, For, Show } from "solid-js";
import { ModpackCard, SearchBox, Spinner, toast, type ModpackHit } from "../components";
import { api } from "../ipc/api";
import type { ProjectKind, SearchHit } from "../ipc/types";
import "./Discover.css";

/**
 * Discover —— Modrinth 搜索页。类型切换 + 防抖搜索 + 卡片网格。
 */

const KINDS: { key: ProjectKind; label: string }[] = [
  { key: "modpack", label: "整合包" },
  { key: "mod", label: "模组" },
  { key: "shader", label: "光影" },
  { key: "resourcepack", label: "资源包" },
];

function toHit(h: SearchHit): ModpackHit {
  return {
    id: (h as any).id ?? h.project_id,
    slug: h.slug,
    title: h.title,
    description: h.description,
    author: h.author,
    downloads: h.downloads,
    icon_url: h.icon_url || undefined,
    gallery_url: (h as any).gallery_url || undefined,
    categories: h.categories,
  };
}

const Discover: Component = () => {
  const [query, setQuery] = createSignal("");
  const [kind, setKind] = createSignal<ProjectKind>("modpack");
  // 防抖后的查询键,作为 resource 的 source。
  const [debounced, setDebounced] = createSignal("");
  let timer: number | undefined;

  function onInput(v: string) {
    setQuery(v);
    clearTimeout(timer);
    timer = setTimeout(() => setDebounced(v), 350) as unknown as number;
  }

  const [results] = createResource(
    () => [debounced(), kind()] as const,
    ([q, k]) =>
      api.modrinthSearch(q, k, null, null).catch((e) => {
        toast({ type: "error", message: `搜索失败:${e}` });
        return [] as SearchHit[];
      }),
  );

  return (
    <div class="discover">
      <div class="discover-head">
        <h1>Discover</h1>
        <SearchBox value={query()} onInput={onInput} placeholder="搜索 Modrinth…" />
      </div>

      <div class="discover-tabs">
        <For each={KINDS}>
          {(k) => (
            <button
              class="discover-tab"
              classList={{ active: kind() === k.key }}
              onClick={() => setKind(k.key)}
            >
              {k.label}
            </button>
          )}
        </For>
      </div>

      <Show when={!results.loading} fallback={<div class="discover-loading"><Spinner /></div>}>
        <Show
          when={(results() ?? []).length > 0}
          fallback={<div class="discover-empty">没有结果,换个关键词试试。</div>}
        >
          <div class="discover-grid">
            <For each={results()}>
              {(hit) => (
                <ModpackCard
                  hit={toHit(hit)}
                  onClick={(h) => toast({ type: "info", message: `打开 ${h.title}` })}
                />
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
};

export default Discover;
