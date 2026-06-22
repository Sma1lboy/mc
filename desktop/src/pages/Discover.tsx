import { Component, createResource, createSignal, For, Show } from "solid-js";
import { ModpackListItem, SearchBox, Spinner, toast, type ModpackHit } from "../components";
import { api } from "../ipc/api";
import type { ProjectKind, SearchHit } from "../ipc/types";
import ModpackDetail from "./ModpackDetail";

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

function isDesktopBackendUnavailable(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes("__TAURI_INTERNALS__") ||
    message.includes("reading 'invoke'") ||
    message.includes("Cannot read properties of undefined")
  );
}

const Discover: Component = () => {
  const [query, setQuery] = createSignal("");
  const [kind, setKind] = createSignal<ProjectKind>("modpack");
  const [backendUnavailable, setBackendUnavailable] = createSignal(false);
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
    async ([q, k]) => {
      setBackendUnavailable(false);
      return api.modrinthSearch(q, k, null, null).catch((e) => {
        if (isDesktopBackendUnavailable(e)) {
          setBackendUnavailable(true);
          return [] as SearchHit[];
        }
        toast({ type: "error", message: `搜索失败:${e}` });
        return [] as SearchHit[];
      });
    },
  );

  // 当前打开详情的整合包(null = 显示搜索网格)。点击卡片进入详情页,而非直接下载。
  const [selected, setSelected] = createSignal<ModpackHit | null>(null);

  function openHit(h: ModpackHit) {
    if (kind() === "modpack") {
      setSelected(h);
    } else {
      // 模组/光影/资源包的详情/安装入口后续接入。
      toast({ type: "info", message: `${h.title}:单资源详情页待接入` });
    }
  }

  return (
    <div class="px-[28px] py-[24px] overflow-y-auto h-full">
      <Show when={selected()}>
        <ModpackDetail hit={selected()!} onBack={() => setSelected(null)} />
      </Show>

      <Show when={!selected()}>
      <div class="flex items-center justify-between gap-[16px] mb-[16px]">
        <h1 class="text-[24px] font-bold text-fg m-0">Discover</h1>
        <SearchBox value={query()} onInput={onInput} placeholder="搜索 Modrinth…" />
      </div>

      <div class="flex gap-[8px] mb-[20px]">
        <For each={KINDS}>
          {(k) => (
            <button
              class="px-[14px] py-[6px] border-none rounded-ctl text-[13px] cursor-pointer transition-[background-color,color,box-shadow] duration-[var(--dur)] ease-app focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5 focus-visible:ring-offset-2 focus-visible:ring-offset-n-1"
              classList={{
                "bg-a-4 text-white": kind() === k.key,
                "bg-n-4 text-dim hover:bg-n-5 hover:text-fg": kind() !== k.key,
              }}
              onClick={() => setKind(k.key)}
            >
              {k.label}
            </button>
          )}
        </For>
      </div>

      <Show when={!results.loading} fallback={<div class="flex justify-center p-[40px]"><Spinner /></div>}>
        <Show
          when={(results() ?? []).length > 0}
          fallback={
            <div class="p-[32px] text-dim text-center">
              <Show
                when={!backendUnavailable()}
                fallback={"浏览器预览未连接桌面后端。打开桌面应用后即可搜索 Modrinth。"}
              >
                {debounced().trim() ? "没有结果,换个关键词试试。" : "输入关键词搜索 Modrinth。"}
              </Show>
            </div>
          }
        >
          <div class="flex flex-col gap-[8px]">
            <For each={results()}>
              {(hit) => (
                <ModpackListItem hit={toHit(hit)} onClick={openHit} />
              )}
            </For>
          </div>
        </Show>
      </Show>
      </Show>
    </div>
  );
};

export default Discover;
