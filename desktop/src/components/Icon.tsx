import { JSX, mergeProps } from "solid-js";

/* ============================================================================
 * components/Icon.tsx —— 统一图标组件(docs/modules/ui-polish.md §3 / §5)
 *
 * 把散落在 ~10 个 TSX 里的内联 <svg> 收口成一个注册表 + <Icon name=… />。
 * 全部用 fill/stroke: currentColor —— 着色跟随 CSS `color`(= Classic `IconBrush`),
 * 尺寸用 width/height(= Classic `Stretch`,配合 viewBox `preserveAspectRatio`)。
 * 颜色过渡是普通 CSS `transition: color`,读 --mo-* 动机令牌(见 Icon.css),
 * 不需任何 JS 动画引擎(对齐 Classic `SvgIcon.AnimateIconBrushTo` 的 120ms 手感)。
 *
 * 渲染模式:
 *   - "stroke" 线性图标:<path> 用 currentColor 描边(line-icon),无填充。
 *   - "fill"   实心图标:<path> 用 currentColor 填充。
 * 每个图标声明 viewBox + 一段 path-data(+ 可选每路径属性,如 dasharray)。
 * ========================================================================== */

/** 单条几何路径 + 其覆盖属性(描边宽度等少数需逐路径定制的场合)。 */
interface IconPath {
  d: string;
}

/** 一个图标的注册项:viewBox、绘制模式、若干路径。 */
interface IconDef {
  /** SVG viewBox(默认 24 网格)。 */
  viewBox: string;
  /** 绘制模式:描边线性 / 实心填充。 */
  mode: "stroke" | "fill";
  /** 线性图标的描边宽度(stroke 模式;默认 1.8)。 */
  strokeWidth?: number;
  /** 路径列表。 */
  paths: IconPath[];
}

/* ----------------------------------------------------------------------------
 * 注册表 —— 收口现有 UI 里在用的图标(Toast 状态 + 通用)。
 * 新增图标:在此追加一项,name 自动并入 IconName 联合类型。
 * -------------------------------------------------------------------------- */
const REGISTRY = {
  /** 电源/启动。 */
  power: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.9,
    paths: [{ d: "M12 3v9" }, { d: "M7.4 6.6a7 7 0 1 0 9.2 0" }],
  },
  /** 下载。 */
  download: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.9,
    paths: [{ d: "M12 3v11" }, { d: "m7.5 9.5 4.5 4.5 4.5-4.5" }, { d: "M5 19.5h14" }],
  },
  /** 设置齿轮。 */
  gear: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.7,
    paths: [
      { d: "M19.4 13a7.8 7.8 0 0 0 0-2l1.8-1.4-1.9-3.3-2.2.9a7.6 7.6 0 0 0-1.7-1l-.3-2.3H10.5l-.3 2.3a7.6 7.6 0 0 0-1.7 1l-2.2-.9-1.9 3.3L6.2 11a7.8 7.8 0 0 0 0 2l-1.8 1.4 1.9 3.3 2.2-.9a7.6 7.6 0 0 0 1.7 1l.3 2.3h3.9l.3-2.3a7.6 7.6 0 0 0 1.7-1l2.2.9 1.9-3.3Z" },
      // 圆心:用一段近似圆的路径,避免混入 <circle>,保持「纯 path」模型。
      { d: "M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6Z" },
    ],
  },
  /** 网格/更多。 */
  grid: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.9,
    paths: [
      { d: "M5.4 4h3.7a1.4 1.4 0 0 1 1.4 1.4v3.7a1.4 1.4 0 0 1-1.4 1.4H5.4A1.4 1.4 0 0 1 4 9.1V5.4A1.4 1.4 0 0 1 5.4 4Z" },
      { d: "M14.9 4h3.7a1.4 1.4 0 0 1 1.4 1.4v3.7a1.4 1.4 0 0 1-1.4 1.4h-3.7a1.4 1.4 0 0 1-1.4-1.4V5.4A1.4 1.4 0 0 1 14.9 4Z" },
      { d: "M5.4 13.5h3.7a1.4 1.4 0 0 1 1.4 1.4v3.7a1.4 1.4 0 0 1-1.4 1.4H5.4A1.4 1.4 0 0 1 4 18.6v-3.7a1.4 1.4 0 0 1 1.4-1.4Z" },
      { d: "M14.9 13.5h3.7a1.4 1.4 0 0 1 1.4 1.4v3.7a1.4 1.4 0 0 1-1.4 1.4h-3.7a1.4 1.4 0 0 1-1.4-1.4v-3.7a1.4 1.4 0 0 1 1.4-1.4Z" },
    ],
  },
  /** 关闭 ✕(Toast 关闭按钮)。 */
  close: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 2,
    paths: [{ d: "M6 6l12 12M18 6 6 18" }],
  },
  /** 勾选(Toast success)。 */
  check: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 2,
    paths: [{ d: "m5 12.5 4.5 4.5L19 7" }],
  },
  /** 信息圆(Toast info)。 */
  info: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.8,
    paths: [
      { d: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z" },
      { d: "M12 11v5M12 7.5h.01" },
    ],
  },
  /** 警告三角(Toast warn)。 */
  warn: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.8,
    paths: [
      { d: "M10.7 3.8a1.5 1.5 0 0 1 2.6 0l7.8 13.8a1.5 1.5 0 0 1-1.3 2.2H4.2a1.5 1.5 0 0 1-1.3-2.2L10.7 3.8Z" },
      { d: "M12 9v4.5M12 16.6h.01" },
    ],
  },
  /** 错误圆(Toast error)。 */
  error: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.8,
    paths: [
      { d: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18Z" },
      { d: "M12 7v6M12 16.6h.01" },
    ],
  },
  /** 搜索放大镜(SearchBox 等通用)。 */
  search: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.9,
    paths: [{ d: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14Z" }, { d: "m20 20-3.5-3.5" }],
  },
  /** 微软四宫格徽标(账号登录:微软正版)。 */
  microsoft: {
    viewBox: "0 0 24 24",
    mode: "fill",
    paths: [
      { d: "M3 3h8.2v8.2H3z" },
      { d: "M12.8 3H21v8.2h-8.2z" },
      { d: "M3 12.8h8.2V21H3z" },
      { d: "M12.8 12.8H21V21h-8.2z" },
    ],
  },
  /** 用户/人像(账号登录:离线账号)。 */
  user: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.8,
    paths: [
      { d: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8Z" },
      { d: "M5 20a7 7 0 0 1 14 0" },
    ],
  },
  /** 多人/好友(好友入口)。 */
  users: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.8,
    paths: [
      { d: "M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" },
      { d: "M2.5 20a6.5 6.5 0 0 1 13 0" },
      { d: "M16 4.2a3.5 3.5 0 0 1 0 6.6" },
      { d: "M17.5 13.6A6.5 6.5 0 0 1 21.5 20" },
    ],
  },
  /** 铃铛/通知(通知中心)。 */
  bell: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.8,
    paths: [
      { d: "M18 9a6 6 0 1 0-12 0c0 5-2 6.5-2 6.5h16S18 14 18 9Z" },
      { d: "M13.7 19a2 2 0 0 1-3.4 0" },
    ],
  },
  /** 链接/外接(账号登录:外置登录)。 */
  link: {
    viewBox: "0 0 24 24",
    mode: "stroke",
    strokeWidth: 1.8,
    paths: [
      { d: "M9.5 17H7.5a5 5 0 0 1 0-10h2" },
      { d: "M14.5 7h2a5 5 0 0 1 0 10h-2" },
      { d: "M8.5 12h7" },
    ],
  },
} satisfies Record<string, IconDef>;

/** 所有已注册图标名(由注册表键推导,新增图标自动并入)。 */
export type IconName = keyof typeof REGISTRY;

/** Icon 组件 props。 */
export interface IconProps {
  /** 图标名(注册表键)。 */
  name: IconName;
  /** 像素尺寸(宽=高),默认 18。 */
  size?: number;
  /** 额外类名(定位/着色 hook)。 */
  class?: string;
  /** 无障碍标签;省略则视为装饰性(aria-hidden)。 */
  label?: string;
}

/**
 * <Icon name="power" />:内联 SVG,currentColor 着色,颜色过渡走 CSS。
 * 描边图标用统一的 round 线帽/线接,与 Toast 视觉一致。
 */
export function Icon(props: IconProps): JSX.Element {
  const merged = mergeProps({ size: 18 }, props);
  const def = (): IconDef => REGISTRY[merged.name];
  const decorative = (): boolean => merged.label === undefined;

  // .ui-icon 的等价 Tailwind:inline-block + shrink-0 + 垂直居中,color 走 currentColor,
  // 颜色/填充/描边过渡读 --mo-dur-fast(120ms,fast 档)+ --mo-ease-out(= ease-emph)。
  const ICON_BASE =
    "inline-block shrink-0 align-middle text-current " +
    "transition-[color,fill,stroke] duration-[var(--mo-dur-fast)] ease-emph";

  return (
    <svg
      class={`${ICON_BASE}${merged.class ? " " + merged.class : ""}`}
      width={merged.size}
      height={merged.size}
      viewBox={def().viewBox}
      fill={def().mode === "fill" ? "currentColor" : "none"}
      stroke={def().mode === "stroke" ? "currentColor" : "none"}
      stroke-width={def().mode === "stroke" ? (def().strokeWidth ?? 1.8) : undefined}
      stroke-linecap="round"
      stroke-linejoin="round"
      role={decorative() ? undefined : "img"}
      aria-hidden={decorative() ? "true" : undefined}
      aria-label={decorative() ? undefined : merged.label}
    >
      {def().paths.map((p) => (
        <path d={p.d} />
      ))}
    </svg>
  );
}

export default Icon;
