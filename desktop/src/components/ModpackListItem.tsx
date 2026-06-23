import { JSX, Show, For } from "solid-js";
import { formatCount } from "./format";
import { t } from "../i18n";
import type { ModpackHit } from "./ModpackCard";

// ModpackListItem —— Discover 的整合包**列表项**(横行,密度高于大卡)。
// 复用 ModpackCard 的 ModpackHit 契约,只是换一种排版:左侧方形缩略图 +
// 中部标题/作者/单行描述/分类 + 右侧下载数 + 可选尾部操作(下载/安装按钮)。
export interface ModpackListItemProps {
  hit: ModpackHit;
  onClick?: (hit: ModpackHit) => void;
  /** 右侧尾部操作槽:渲染在下载数之后(如「安装」按钮)。点击不应冒泡到行 onClick。 */
  action?: JSX.Element;
}

export function ModpackListItem(props: ModpackListItemProps): JSX.Element {
  const hit = () => props.hit;

  const initial = () => {
    const t = hit().title?.trim();
    return t && t.length > 0 ? t[0] : "?";
  };

  // 列表项用方形 icon(横版封面在窄行里太宽),缺失再退回横版封面。
  const thumb = () => hit().icon_url || hit().gallery_url;

  // 列表更宽,容得下 4 个分类。
  const chips = () => (hit().categories ?? []).slice(0, 4);

  return (
    <div
      class="glass-card glass-card--hover flex items-center gap-[14px] px-[14px] py-[10px] rounded-card cursor-pointer transition-[background-color,border-color,transform,box-shadow] duration-[var(--dur)] ease-app hover:translate-x-[2px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
      role={props.onClick ? "button" : undefined}
      tabindex={props.onClick ? 0 : undefined}
      onClick={() => props.onClick?.(hit())}
      onKeyDown={(e) => {
        if (props.onClick && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          props.onClick(hit());
        }
      }}
    >
      {/* 左:方形缩略图,缺失 → 渐变 + 首字母。 */}
      <div class="relative w-[60px] h-[60px] flex-[0_0_auto] rounded-[12px] overflow-hidden flex items-center justify-center bg-[linear-gradient(135deg,var(--a-2),var(--a-4)_60%,var(--a-5))]">
        <Show
          when={thumb()}
          fallback={
            <span class="text-[rgba(255,255,255,0.88)] text-[26px] font-extrabold uppercase select-none">
              {initial()}
            </span>
          }
        >
          <img src={thumb()} alt="" width="60" height="60" loading="lazy" class="w-full h-full object-cover block" />
        </Show>
      </div>

      {/* 中:标题 + 作者 / 单行描述 / 分类。 */}
      <div class="flex-1 min-w-0 flex flex-col gap-[3px]">
        <div class="flex items-baseline gap-[8px] min-w-0">
          <span
            class="text-[15px] font-bold text-fg whitespace-nowrap overflow-hidden text-ellipsis"
            title={hit().title}
          >
            {hit().title}
          </span>
          <Show when={hit().author}>
            <span class="flex-[0_0_auto] text-[12px] text-dim">by {hit().author}</span>
          </Show>
        </div>
        <Show when={hit().description}>
          <div class="text-[13px] leading-[1.45] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
            {hit().description}
          </div>
        </Show>
        <Show when={chips().length}>
          <div class="flex gap-[5px] overflow-hidden flex-nowrap mt-[1px]">
            <For each={chips()}>
              {(c) => (
                <span class="text-[11px] text-dim bg-glass-card rounded-xs px-[7px] py-[2px] whitespace-nowrap capitalize">
                  {c}
                </span>
              )}
            </For>
          </div>
        </Show>
      </div>

      {/* 右:下载数 + 可选操作(安装/下载)。 */}
      <div class="flex-[0_0_auto] flex items-center gap-[10px]">
        <span
          class="inline-flex items-center gap-[5px] text-[13px] text-dim [font-variant-numeric:tabular-nums]"
          title={t("discover.downloadsTooltip", { count: hit().downloads })}
        >
          <svg
            width="13"
            height="13"
            viewBox="0 0 14 14"
            fill="currentColor"
            aria-hidden="true"
            class="text-a-5"
          >
            <path d="M7 1a.9.9 0 0 1 .9.9v5.04l1.5-1.5a.9.9 0 1 1 1.27 1.27L7.64 9.94a.9.9 0 0 1-1.28 0L3.33 6.71A.9.9 0 0 1 4.6 5.44l1.5 1.5V1.9A.9.9 0 0 1 7 1Z" />
            <path d="M2.1 10.2a.9.9 0 0 1 .9.9v.7h8v-.7a.9.9 0 1 1 1.8 0v1.1a1.4 1.4 0 0 1-1.4 1.4H2.7a1.4 1.4 0 0 1-1.4-1.4v-1.1a.9.9 0 0 1 .9-.9Z" />
          </svg>
          {formatCount(hit().downloads)}
        </span>
        {/* 尾部操作:阻止冒泡,避免点按钮同时触发整行的打开详情。 */}
        <Show when={props.action}>
          <div onClick={(e) => e.stopPropagation()}>{props.action}</div>
        </Show>
      </div>
    </div>
  );
}

export default ModpackListItem;
