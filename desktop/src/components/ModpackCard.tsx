import { JSX, Show, For } from "solid-js";
import { formatCount } from "./format";

// ModpackCard 接收的搜索命中形状。与后端 SearchHit 字段对齐。
export interface ModpackHit {
  id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  downloads: number;
  icon_url?: string;
  /** 高清横版封面(Modrinth gallery / featured),优先于 icon_url。 */
  gallery_url?: string;
  categories: string[];
}

// ModpackCard —— Discover 的整合包大卡。
// props 契约:
//   hit: 搜索命中数据
//   onClick?: 点卡片回调 (传入 hit, 供页面打开详情)
export interface ModpackCardProps {
  hit: ModpackHit;
  onClick?: (hit: ModpackHit) => void;
}

export function ModpackCard(props: ModpackCardProps): JSX.Element {
  const hit = () => props.hit;

  const initial = () => {
    const t = hit().title?.trim();
    return t && t.length > 0 ? t[0] : "?";
  };

  // 最多展示 3 个分类 chip, 避免溢出。
  const chips = () => (hit().categories ?? []).slice(0, 3);

  // 封面优先用高清横版 gallery,缺失再退回方形 icon。
  const cover = () => hit().gallery_url || hit().icon_url;

  return (
    <div
      class={
        "flex flex-col bg-card rounded-card shadow-card border border-transparent " +
        "overflow-hidden cursor-pointer " +
        "transition-[transform,box-shadow,border-color] duration-[var(--dur)] ease-app " +
        "hover:-translate-y-[3px] hover:shadow-[0_8px_24px_rgba(0,0,0,0.45)] hover:border-n-6"
      }
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
      {/* 顶部封面 16:9: 有 icon_url 显示, 否则渐变 + 大首字母占位。 */}
      <div class="relative w-full aspect-[16/9] overflow-hidden bg-[linear-gradient(135deg,var(--a-2),var(--a-4)_60%,var(--a-5))]">
        <Show
          when={cover()}
          fallback={
            <span class="absolute inset-0 flex items-center justify-center text-[rgba(255,255,255,0.85)] text-[42px] font-extrabold uppercase select-none">
              {initial()}
            </span>
          }
        >
          <img
            src={cover()}
            alt=""
            loading="lazy"
            class="absolute inset-0 block w-full h-full object-cover"
          />
        </Show>
      </div>

      <div class="flex flex-col flex-1 gap-[8px] p-[14px]">
        {/* 标题 + 作者 (作者灰色小字接在标题后)。 */}
        <div
          class="text-[15px] font-bold text-fg whitespace-nowrap overflow-hidden text-ellipsis"
          title={hit().title}
        >
          {hit().title}
          <Show when={hit().author}>
            <span class="text-dim font-normal"> by {hit().author}</span>
          </Show>
        </div>

        {/* 描述 2 行截断, min-height 固定避免高度跳动。 */}
        <div class="text-[13px] leading-[1.45] text-dim line-clamp-2 min-h-[calc(1.45em*2)]">
          {hit().description}
        </div>

        {/* 统计行: 下载数 (k/M 缩写) + 分类标签。 */}
        <div class="flex items-center flex-wrap gap-[8px] mt-auto">
          <span
            class="inline-flex items-center shrink-0 gap-[4px] text-[12px] text-dim"
            title={`${hit().downloads} downloads`}
          >
            {/* 下载图标 (向下箭头入托盘), accent 色。 */}
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

          <div class="flex items-center flex-nowrap gap-[5px] overflow-hidden">
            <For each={chips()}>
              {(cat) => (
                <span class="text-[11px] text-dim bg-n-5 rounded-xs px-[7px] py-[2px] whitespace-nowrap capitalize">
                  {cat}
                </span>
              )}
            </For>
          </div>
        </div>
      </div>
    </div>
  );
}

export default ModpackCard;
