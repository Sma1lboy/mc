import { JSX, Show, For } from "solid-js";
import { formatCount } from "./format";
import "./ModpackCard.css";

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
      class="ui-mpcard"
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
      {/* 顶部封面: 有 icon_url 显示, 否则渐变 + 大首字母占位。 */}
      <div class="ui-mpcard__cover">
        <Show
          when={cover()}
          fallback={<span class="ui-mpcard__cover-letter">{initial()}</span>}
        >
          <img src={cover()} alt="" loading="lazy" />
        </Show>
      </div>

      <div class="ui-mpcard__body">
        {/* 标题 + 作者 (作者灰色小字接在标题后)。 */}
        <div class="ui-mpcard__title" title={hit().title}>
          {hit().title}
          <Show when={hit().author}>
            <span class="ui-mpcard__author"> by {hit().author}</span>
          </Show>
        </div>

        {/* 描述 2 行截断。 */}
        <div class="ui-mpcard__desc">{hit().description}</div>

        {/* 统计行: 下载数 (k/M 缩写) + 分类标签。 */}
        <div class="ui-mpcard__stats">
          <span class="ui-mpcard__downloads" title={`${hit().downloads} downloads`}>
            {/* 下载图标 (向下箭头入托盘)。 */}
            <svg width="13" height="13" viewBox="0 0 14 14" fill="currentColor" aria-hidden="true">
              <path d="M7 1a.9.9 0 0 1 .9.9v5.04l1.5-1.5a.9.9 0 1 1 1.27 1.27L7.64 9.94a.9.9 0 0 1-1.28 0L3.33 6.71A.9.9 0 0 1 4.6 5.44l1.5 1.5V1.9A.9.9 0 0 1 7 1Z" />
              <path d="M2.1 10.2a.9.9 0 0 1 .9.9v.7h8v-.7a.9.9 0 1 1 1.8 0v1.1a1.4 1.4 0 0 1-1.4 1.4H2.7a1.4 1.4 0 0 1-1.4-1.4v-1.1a.9.9 0 0 1 .9-.9Z" />
            </svg>
            {formatCount(hit().downloads)}
          </span>

          <div class="ui-mpcard__chips">
            <For each={chips()}>
              {(cat) => <span class="ui-mpcard__chip">{cat}</span>}
            </For>
          </div>
        </div>
      </div>
    </div>
  );
}

export default ModpackCard;
