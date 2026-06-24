import {
  Component,
  createMemo,
  createResource,
  createSignal,
  For,
  Show,
} from "solid-js";
import { api } from "../ipc/api";
import { t } from "../i18n";
import type { ProjectKind, CategoryTag, LoaderTag, GameVersionTag } from "../ipc/types";
import type { ContentProvider } from "./ContentBrowser";
import { SearchBox } from "./SearchBox";
import { Spinner } from "./Spinner";

/**
 * FacetSidebar —— Discover 浏览态的 Modrinth 多选 facet 过滤侧栏。
 *
 * 分类法(分类 / loader / 游戏版本)来自 `content_facets` 命令(进程内缓存),
 * 仅 Modrinth 提供;CurseForge 选中时只显示一条说明(后端忽略这些 facet)。
 *
 * 渲染按 *当前类型* 裁剪:
 *   - 内容分类按 `header` 分组(categories / features / resolutions / performance impact),
 *     只取 `project_type` 与当前类型匹配的项(datapack 在 Modrinth 是 `mod` 项目)。
 *   - 运行环境(客户端 / 服务端):mod / modpack 才显示。
 *   - 游戏版本:可搜索 + 「显示全部版本」开关(默认只 release;开启含快照),多选。
 *   - Loader(Fabric/Forge/…):mod 才显示,取 supported_project_types 含当前类型的 loader。
 *
 * 分类名直接来自 Modrinth(英文)——原样展示,不走 i18n;分组标题 / 开关 / 清除 走 t()。
 */

/** Discover 维护的 facet 选择状态(多选)。 */
export interface FacetSelection {
  categories: string[];
  loaders: string[];
  gameVersions: string[];
  environment: string | null;
  /** License:仅开源项目。 */
  openSource: boolean;
}

export interface FacetSidebarProps {
  kind: ProjectKind;
  provider: ContentProvider;
  selected: () => FacetSelection;
  onChange: (next: FacetSelection) => void;
}

/** 把前端 ProjectKind 映射到 Modrinth 的 project_type(datapack 归入 mod)。 */
function modrinthProjectType(kind: ProjectKind): string {
  return kind === "datapack" ? "mod" : kind;
}

/** content header → i18n 分组标题键。 */
function headerTitleKey(header: string): string {
  switch (header) {
    case "categories":
      return "facets.headerCategories";
    case "features":
      return "facets.headerFeatures";
    case "resolutions":
      return "facets.headerResolutions";
    case "performance impact":
      return "facets.headerPerformance";
    default:
      return "facets.headerCategories";
  }
}

// 内容分类分组的渲染顺序(与 Modrinth header 一致)。
const CONTENT_HEADERS = ["categories", "features", "resolutions", "performance impact"];

/** 一行多选项:复选框 + 标签。house glass 风格。 */
const FacetCheckbox: Component<{
  label: string;
  checked: boolean;
  onToggle: () => void;
}> = (props) => (
  <button
    type="button"
    role="checkbox"
    aria-checked={props.checked}
    onClick={props.onToggle}
    class="flex items-center gap-[8px] w-full text-left px-[8px] py-[5px] rounded-ctl border-none bg-transparent cursor-pointer text-[12px] transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-glass-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
    classList={{ "text-fg": props.checked, "text-dim": !props.checked }}
  >
    <span
      class="shrink-0 inline-flex items-center justify-center w-[15px] h-[15px] rounded-[4px] border transition-[background-color,border-color] duration-[var(--dur)] ease-app"
      classList={{
        "bg-a-4 border-a-4 text-white": props.checked,
        "border-glass-border-strong bg-transparent": !props.checked,
      }}
    >
      <Show when={props.checked}>
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none" aria-hidden="true">
          <path d="m2.5 6.2 2.3 2.3L9.5 3.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </Show>
    </span>
    <span class="min-w-0 truncate">{props.label}</span>
  </button>
);

/** 可折叠分组容器(glass 卡片)。 */
const FacetSection: Component<{
  title: string;
  count?: number;
  children: import("solid-js").JSX.Element;
}> = (props) => {
  const [open, setOpen] = createSignal(true);
  return (
    <div class="glass-input rounded-ctl border border-glass-border overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        class="flex items-center justify-between w-full px-[10px] py-[8px] border-none bg-transparent cursor-pointer text-[12px] font-semibold text-fg transition-colors duration-[var(--dur)] ease-app hover:bg-glass-hover focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
      >
        <span class="inline-flex items-center gap-[6px]">
          {props.title}
          <Show when={props.count}>
            <span class="inline-flex items-center justify-center min-w-[16px] h-[16px] px-[4px] rounded-full bg-a-4 text-white text-[10px] font-semibold">
              {props.count}
            </span>
          </Show>
        </span>
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          class="shrink-0 text-dim transition-transform duration-[var(--dur)] ease-app"
          classList={{ "rotate-180": open() }}
          aria-hidden="true"
        >
          <path d="m4 5.5 3 3 3-3" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      <Show when={open()}>
        <div class="px-[6px] pb-[6px] flex flex-col gap-[1px]">{props.children}</div>
      </Show>
    </div>
  );
};

export const FacetSidebar: Component<FacetSidebarProps> = (props) => {
  // 分类法缓存:进程内 OnceCell(后端),前端 createResource 只在首挂时取一次。
  const [facets] = createResource(() => api.contentFacets());

  // 仅 Modrinth 支持 facet 过滤;CF 选中时只给说明。
  const isModrinth = () => props.provider === "modrinth";

  // 当前类型对应的 Modrinth project_type。
  const ptype = createMemo(() => modrinthProjectType(props.kind));

  // 内容分类(按 header 分组,过滤当前类型)。
  const contentGroups = createMemo(() => {
    const data = facets();
    if (!data) return [] as { header: string; items: CategoryTag[] }[];
    const pt = ptype();
    const byHeader = new Map<string, CategoryTag[]>();
    for (const c of data.categories) {
      if (c.project_type !== pt) continue;
      // datapack 列表里隐藏 "datapack" 这个自指分类(它由后端按类型自动追加)。
      if (props.kind === "datapack" && c.name === "datapack") continue;
      const arr = byHeader.get(c.header) ?? [];
      arr.push(c);
      byHeader.set(c.header, arr);
    }
    return CONTENT_HEADERS.filter((h) => byHeader.has(h)).map((h) => ({
      header: h,
      items: byHeader.get(h)!,
    }));
  });

  // Loader(仅 mod;取 supported_project_types 含 mod 的)。
  const loaderList = createMemo(() => {
    const data = facets();
    if (!data || props.kind !== "mod") return [] as LoaderTag[];
    return data.loaders.filter((l) => l.supported_project_types.includes("mod"));
  });

  // 游戏版本:搜索 + release/全部开关。
  const [versionQuery, setVersionQuery] = createSignal("");
  const [showAllVersions, setShowAllVersions] = createSignal(false);
  const versionList = createMemo(() => {
    const data = facets();
    if (!data) return [] as GameVersionTag[];
    const q = versionQuery().trim().toLowerCase();
    return data.game_versions.filter((v) => {
      if (!showAllVersions() && v.version_type !== "release") return false;
      if (q && !v.version.toLowerCase().includes(q)) return false;
      return true;
    });
  });

  // 环境过滤仅对 mod / modpack 有意义。
  const showEnvironment = () => props.kind === "mod" || props.kind === "modpack";
  const showLoaders = () => props.kind === "mod";

  const sel = () => props.selected();

  function toggleIn(list: string[], value: string): string[] {
    return list.includes(value) ? list.filter((x) => x !== value) : [...list, value];
  }

  function toggleCategory(name: string) {
    props.onChange({ ...sel(), categories: toggleIn(sel().categories, name) });
  }
  function toggleLoader(name: string) {
    props.onChange({ ...sel(), loaders: toggleIn(sel().loaders, name) });
  }
  function toggleVersion(version: string) {
    props.onChange({ ...sel(), gameVersions: toggleIn(sel().gameVersions, version) });
  }
  function setEnvironment(env: string) {
    const next = sel().environment === env ? null : env;
    props.onChange({ ...sel(), environment: next });
  }
  function toggleOpenSource() {
    props.onChange({ ...sel(), openSource: !sel().openSource });
  }

  const activeCount = createMemo(() => {
    const s = sel();
    return s.categories.length + s.loaders.length + s.gameVersions.length + (s.environment ? 1 : 0) + (s.openSource ? 1 : 0);
  });

  function clearAll() {
    props.onChange({ categories: [], loaders: [], gameVersions: [], environment: null, openSource: false });
  }

  return (
    <aside class="shrink-0 w-[248px] flex flex-col gap-[10px] overflow-y-auto pr-[2px]">
      <div class="flex items-center justify-between gap-[8px]">
        <h2 class="text-[13px] font-semibold text-fg m-0">{t("facets.title")}</h2>
        <Show when={activeCount() > 0}>
          <button
            type="button"
            onClick={clearAll}
            class="text-[11px] text-a-6 bg-transparent border-none cursor-pointer hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5 rounded-xs px-[2px]"
          >
            {t("facets.clear")}
          </button>
        </Show>
      </div>

      <Show
        when={isModrinth()}
        fallback={
          <div class="glass-input rounded-ctl border border-glass-border px-[10px] py-[10px] text-[12px] text-dim leading-relaxed">
            {t("facets.cfNoFacets")}
          </div>
        }
      >
        <Show
          when={facets()}
          fallback={
            <Show
              when={!facets.error}
              fallback={<div class="text-[12px] text-dim px-[4px] py-[8px]">{t("facets.loadFailed")}</div>}
            >
              <div class="flex justify-center py-[20px]"><Spinner /></div>
            </Show>
          }
        >
          {/* 内容分类(按 header 分组)。 */}
          <For each={contentGroups()}>
            {(group) => {
              const selectedCount = () =>
                group.items.filter((c) => sel().categories.includes(c.name)).length;
              return (
                <FacetSection title={t(headerTitleKey(group.header))} count={selectedCount()}>
                  <For each={group.items}>
                    {(c) => (
                      <FacetCheckbox
                        label={c.name}
                        checked={sel().categories.includes(c.name)}
                        onToggle={() => toggleCategory(c.name)}
                      />
                    )}
                  </For>
                </FacetSection>
              );
            }}
          </For>

          {/* 运行环境(mod / modpack)。 */}
          <Show when={showEnvironment()}>
            <FacetSection title={t("facets.headerEnvironment")} count={sel().environment ? 1 : 0}>
              <FacetCheckbox
                label={t("facets.envClient")}
                checked={sel().environment === "client"}
                onToggle={() => setEnvironment("client")}
              />
              <FacetCheckbox
                label={t("facets.envServer")}
                checked={sel().environment === "server"}
                onToggle={() => setEnvironment("server")}
              />
            </FacetSection>
          </Show>

          {/* 加载器(mod)。 */}
          <Show when={showLoaders() && loaderList().length > 0}>
            <FacetSection
              title={t("facets.headerLoader")}
              count={loaderList().filter((l) => sel().loaders.includes(l.name)).length}
            >
              <For each={loaderList()}>
                {(l) => (
                  <FacetCheckbox
                    label={l.name}
                    checked={sel().loaders.includes(l.name)}
                    onToggle={() => toggleLoader(l.name)}
                  />
                )}
              </For>
            </FacetSection>
          </Show>

          {/* 游戏版本(搜索 + release/全部开关)。 */}
          <FacetSection title={t("facets.headerGameVersion")} count={sel().gameVersions.length}>
            <div class="px-[2px] pt-[2px] pb-[4px] flex flex-col gap-[6px]">
              <SearchBox
                value={versionQuery()}
                onInput={setVersionQuery}
                placeholder={t("facets.versionSearchPlaceholder")}
                class="!h-[30px]"
              />
              <label class="flex items-center gap-[6px] px-[6px] text-[11px] text-dim cursor-pointer select-none">
                <input
                  type="checkbox"
                  class="accent-a-4 cursor-pointer"
                  checked={showAllVersions()}
                  onChange={(e) => setShowAllVersions(e.currentTarget.checked)}
                />
                {t("facets.showAllVersions")}
              </label>
            </div>
            <div class="max-h-[220px] overflow-y-auto flex flex-col gap-[1px]">
              <For each={versionList()}>
                {(v) => (
                  <FacetCheckbox
                    label={v.version}
                    checked={sel().gameVersions.includes(v.version)}
                    onToggle={() => toggleVersion(v.version)}
                  />
                )}
              </For>
            </div>
          </FacetSection>

          {/* License:仅开源(始终置底,各类型一致)。 */}
          <FacetSection title={t("facets.headerLicense")} count={sel().openSource ? 1 : 0}>
            <FacetCheckbox
              label={t("facets.openSource")}
              checked={sel().openSource}
              onToggle={toggleOpenSource}
            />
          </FacetSection>
        </Show>
      </Show>
    </aside>
  );
};

export default FacetSidebar;
