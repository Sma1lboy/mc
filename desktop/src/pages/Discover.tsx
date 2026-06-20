import { Component, createResource, createSignal, For, Show } from "solid-js";
import { ModpackCard, SearchBox, Spinner, toast, type ModpackHit } from "../components";
import { api } from "../ipc/api";
import { currentRoot } from "../store";
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

  // 正在安装的整合包 project id(防重复点击)。
  const [installing, setInstalling] = createSignal<string | null>(null);

  async function openHit(h: ModpackHit) {
    // 模组/光影/资源包暂只提示(安装入口在版本详情,后续接入);整合包则直接安装。
    if (kind() !== "modpack") {
      toast({ type: "info", message: `${h.title}:在「整合包」标签可一键安装;单资源安装入口待接入` });
      return;
    }
    if (installing()) {
      toast({ type: "info", message: "已有整合包正在安装,请稍候…" });
      return;
    }
    setInstalling(h.id);
    toast({ type: "info", message: `开始安装「${h.title}」…首次会下载原版与依赖,可能需要几分钟` });
    try {
      const out = await api.installModrinthModpack(currentRoot() ?? "", h.id, null);
      const blocked = out.blocked.length;
      toast({
        type: blocked > 0 ? "info" : "success",
        message:
          blocked > 0
            ? `已安装「${out.instance_id}」(${blocked} 个文件需手动下载),去启动页选择它`
            : `已安装整合包「${out.instance_id}」,去启动页选择它即可开玩`,
      });
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
    } finally {
      setInstalling(null);
    }
  }

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
                <ModpackCard hit={toHit(hit)} onClick={openHit} />
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
};

export default Discover;
