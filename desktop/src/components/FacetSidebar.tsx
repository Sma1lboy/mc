import {
  Component,
  createMemo,
  createSignal,
  For,
  Show,
} from "solid-js";
import { t } from "../i18n";
import type { ProjectKind, CategoryTag, LoaderTag, GameVersionTag, FacetTagsDto } from "../ipc/types";
import type { ContentProvider } from "./ContentBrowser";
import { SearchBox } from "./SearchBox";
import { Chip } from "./Chip";

/**
 * FacetSidebar —— Discover 浏览态的 Modrinth 多选 facet 过滤。
 *
 * Blocky Craft 改造:从「整列侧栏」改为「更多筛选」弹层的**内容体**。
 * 顶部一行可移除筛选 Chips(由 ContentBrowser 用 {@link facetChips} 渲染)+ 一个
 * 「更多筛选」入口,点开后弹出本 <FacetPanel>:分类 / 运行环境 / 加载器 / 游戏版本 /
 * 许可证 的多选清单。所有 facet 选择逻辑、按类型裁剪规则、CF 不支持提示均保留。
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
  /** 分类法:由 Discover 统一拉取并下传(整体就绪后再渲染,弹层不单独等待)。 */
  tags: () => FacetTagsDto | undefined;
}

/** 一条已选 facet 的可移除芯片描述(供 ContentBrowser 渲染顶部筛选条)。 */
export interface FacetChip {
  /** 跨维度唯一(用作 For key)。 */
  key: string;
  /** 展示文案(分类原样、环境/开源走 i18n)。 */
  label: string;
  /** 移除该项,返回更新后的 selection。 */
  remove: (sel: FacetSelection) => FacetSelection;
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

/** API 返回小写 slug(如 "kitchen-sink");展示时首字母大写、`-`/`_` 转空格。 */
function titleCase(slug: string): string {
  return slug
    .split(/[-_]/)
    .map((w) => (w ? w[0].toUpperCase() + w.slice(1) : w))
    .join(" ");
}

/**
 * 把当前 facet 选择展开成一行可移除芯片(供 ContentBrowser 顶部筛选条)。
 * 顺序:分类 → 加载器 → 游戏版本 → 运行环境 → 开源。每条带去重 key + 移除函数。
 */
export function facetChips(sel: FacetSelection): FacetChip[] {
  const chips: FacetChip[] = [];
  for (const name of sel.categories) {
    chips.push({
      key: `cat:${name}`,
      label: titleCase(name),
      remove: (s) => ({ ...s, categories: s.categories.filter((x) => x !== name) }),
    });
  }
  for (const name of sel.loaders) {
    chips.push({
      key: `loader:${name}`,
      label: titleCase(name),
      remove: (s) => ({ ...s, loaders: s.loaders.filter((x) => x !== name) }),
    });
  }
  for (const v of sel.gameVersions) {
    chips.push({
      key: `ver:${v}`,
      label: v,
      remove: (s) => ({ ...s, gameVersions: s.gameVersions.filter((x) => x !== v) }),
    });
  }
  if (sel.environment) {
    const env = sel.environment;
    chips.push({
      key: `env:${env}`,
      label: t(env === "client" ? "facets.envClient" : "facets.envServer"),
      remove: (s) => ({ ...s, environment: null }),
    });
  }
  if (sel.openSource) {
    chips.push({
      key: "license:open",
      label: t("facets.openSource"),
      remove: (s) => ({ ...s, openSource: false }),
    });
  }
  return chips;
}

/** 已选 facet 总数(供「更多筛选」入口显示徽标 / 是否高亮)。 */
export function facetCount(sel: FacetSelection): number {
  return (
    sel.categories.length +
    sel.loaders.length +
    sel.gameVersions.length +
    (sel.environment ? 1 : 0) +
    (sel.openSource ? 1 : 0)
  );
}

/** 一行多选项:方块复选框 + 标签。 */
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
    class="flex items-center gap-[8px] w-full text-left px-[8px] py-[5px] rounded-none border-none bg-transparent cursor-pointer text-[12px] transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-panel-3 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
    classList={{ "text-fg": props.checked, "text-muted": !props.checked }}
  >
    <span
      class="shrink-0 inline-flex items-center justify-center w-[16px] h-[16px] rounded-none transition-[background-color] duration-[var(--dur)] ease-app"
      classList={{
        "bg-accent text-accent-text shadow-raised": props.checked,
        "bg-panel-2 shadow-input": !props.checked,
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

/** 可折叠分组容器(凹陷小面板)。 */
const FacetSection: Component<{
  title: string;
  count?: number;
  children: import("solid-js").JSX.Element;
}> = (props) => {
  const [open, setOpen] = createSignal(true);
  return (
    <div class="bg-panel-2 shadow-input rounded-none overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        class="flex items-center justify-between w-full px-[10px] py-[8px] border-none bg-transparent cursor-pointer text-[12px] font-semibold text-fg transition-colors duration-[var(--dur)] ease-app hover:bg-panel-3 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      >
        <span class="inline-flex items-center gap-[6px]">
          {props.title}
          <Show when={props.count}>
            <span class="inline-flex items-center justify-center min-w-[16px] h-[16px] px-[4px] rounded-none bg-accent text-accent-text text-[10px] font-semibold shadow-raised">
              {props.count}
            </span>
          </Show>
        </span>
        <svg
          width="14"
          height="14"
          viewBox="0 0 14 14"
          fill="none"
          class="shrink-0 text-muted transition-transform duration-[var(--dur)] ease-app"
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

/**
 * FacetSidebar —— 现作为「更多筛选」弹层的内容体渲染(由 ContentBrowser 放进 Menu/弹层)。
 * 名字保留以兼容现有导入;内部不再是 <aside> 侧栏,而是一个垂直滚动的面板内容。
 */
export const FacetSidebar: Component<FacetSidebarProps> = (props) => {
  // 分类法由 Discover 统一拉取下传(整体就绪后再渲染),弹层不单独 fetch/等待。
  const facets = () => props.tags();

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

  const activeCount = createMemo(() => facetCount(sel()));

  function clearAll() {
    props.onChange({ categories: [], loaders: [], gameVersions: [], environment: null, openSource: false });
  }

  return (
    // Discover 浏览态:左侧固定宽筛选栏(整列随页面一起滚动,简单稳妥)。
    <aside class="w-[240px] shrink-0 flex flex-col gap-[10px]">
      <div class="flex items-center justify-between gap-[8px]">
        <h2 class="text-[13px] font-semibold text-strong m-0">{t("facets.title")}</h2>
        <Show when={activeCount() > 0}>
          <Chip onClick={clearAll} class="!h-[24px] !px-[8px] !text-[11px]">
            {t("facets.clear")}
          </Chip>
        </Show>
      </div>

      <Show
        when={isModrinth()}
        fallback={
          <div class="bg-panel-2 shadow-input rounded-none px-[10px] py-[10px] text-[12px] text-muted leading-relaxed">
            {t("facets.cfNoFacets")}
          </div>
        }
      >
        <Show
          when={facets()}
          fallback={<div class="text-[12px] text-muted px-[4px] py-[8px]">{t("facets.loadFailed")}</div>}
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
                        label={titleCase(c.name)}
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
                    label={titleCase(l.name)}
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
              <label class="flex items-center gap-[6px] px-[6px] text-[11px] text-muted cursor-pointer select-none">
                <input
                  type="checkbox"
                  class="accent-[var(--accent)] cursor-pointer"
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
