import { formatCount } from "./format";
import { Tag, PixelLabel } from ".";
import { t } from "../i18n";

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

export function ModpackCard(props: ModpackCardProps): React.ReactElement {
  const hit = props.hit;

  const s = hit.title?.trim();
  const initial = s && s.length > 0 ? s[0] : "?";

  // 最多展示 3 个分类 chip, 避免溢出。
  const chips = (hit.categories ?? []).slice(0, 3);

  // 封面优先用高清横版 gallery,缺失再退回方形 icon。
  const cover = hit.gallery_url || hit.icon_url;

  return (
    <div
      className={
        "group flex flex-col bg-panel shadow-sunken rounded-none " +
        "overflow-hidden cursor-pointer " +
        "transition-[transform] duration-[var(--dur)] ease-app " +
        "hover:-translate-y-[3px] " +
        "focus-visible:outline-none focus-visible:shadow-raised"
      }
      role={props.onClick ? "button" : undefined}
      tabIndex={props.onClick ? 0 : undefined}
      onClick={() => props.onClick?.(hit)}
      onKeyDown={(e) => {
        if (props.onClick && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          props.onClick(hit);
        }
      }}
    >
      {/* 顶部封面 16:9: 有图显示, 否则草方块色块 + 大首字母占位。 */}
      <div
        className="relative w-full aspect-[16/9] overflow-hidden shadow-input"
        style={{
          background: "linear-gradient(var(--grass-top) 0 42%, var(--grass-side) 42% 100%)",
        }}
      >
        {cover ? (
          <img
            src={cover}
            alt=""
            width="320"
            height="180"
            loading="lazy"
            className="absolute inset-0 block w-full h-full object-cover"
          />
        ) : (
          <span className="absolute inset-0 flex items-center justify-center font-display text-strong text-[44px] uppercase select-none drop-shadow-[0_2px_0_rgba(0,0,0,0.35)]">
            {initial}
          </span>
        )}
      </div>

      <div className="flex flex-col flex-1 gap-[8px] p-[14px]">
        {/* 标题 (像素体) + 作者 (灰色小字接在标题后)。 */}
        <div
          className="font-display text-[16px] text-strong whitespace-nowrap overflow-hidden text-ellipsis"
          title={hit.title}
        >
          {hit.title}
          {hit.author && <span className="font-sans text-[12px] text-muted"> · {hit.author}</span>}
        </div>

        {/* 描述 2 行截断, min-height 固定避免高度跳动。 */}
        <div className="text-[13px] leading-[1.45] text-sub line-clamp-2 min-h-[calc(1.45em*2)]">
          {hit.description}
        </div>

        {/* 统计行: 下载数 (点阵数字) + 分类标签。 */}
        <div className="flex items-center flex-wrap gap-[8px] mt-auto">
          <span
            className="inline-flex items-center shrink-0 gap-[5px] text-accent"
            title={t("discover.downloadsTooltip", { count: hit.downloads })}
          >
            {/* 下载图标 (向下箭头入托盘)。 */}
            <svg width="13" height="13" viewBox="0 0 14 14" fill="currentColor" aria-hidden="true">
              <path d="M7 1a.9.9 0 0 1 .9.9v5.04l1.5-1.5a.9.9 0 1 1 1.27 1.27L7.64 9.94a.9.9 0 0 1-1.28 0L3.33 6.71A.9.9 0 0 1 4.6 5.44l1.5 1.5V1.9A.9.9 0 0 1 7 1Z" />
              <path d="M2.1 10.2a.9.9 0 0 1 .9.9v.7h8v-.7a.9.9 0 1 1 1.8 0v1.1a1.4 1.4 0 0 1-1.4 1.4H2.7a1.4 1.4 0 0 1-1.4-1.4v-1.1a.9.9 0 0 1 .9-.9Z" />
            </svg>
            <PixelLabel className="text-[9px] text-accent">{formatCount(hit.downloads)}</PixelLabel>
          </span>

          <div className="flex items-center flex-nowrap gap-[5px] overflow-hidden">
            {chips.map((cat) => (
              <Tag key={cat} className="capitalize">
                {cat}
              </Tag>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

export default ModpackCard;
