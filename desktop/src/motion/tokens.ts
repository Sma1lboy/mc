/* ============================================================================
 * motion/tokens.ts —— 动机令牌的 TS 镜像
 *
 * 与 theme/tokens.css 里的 --mo-* 变量「同名同值」,让 CSS 路径(transition/
 * keyframes 引用 var(--mo-*))与 JS 路径(rAF/WAAPI 引用这里的常量)共享同一
 * 个真相源(docs/modules/ui-animation.md §4)。改 PCL「手感」只需改这两处之一
 * 并保持一致。
 *
 * 数值单位:DUR 为毫秒(number),EASE 为 cubic-bezier 字符串(供 WAAPI/CSS)。
 * JS 缓动的纯函数版本在 easings.ts(cubic-bezier 表达不了的 overshoot/elastic)。
 * ========================================================================== */

/** 命名时长尺(毫秒)。镜像 --mo-dur-*。 */
export const DUR = {
  /** 80ms —— 瞬时反馈(按下挤压)。 */
  instant: 80,
  /** 150ms —— 快(勾选/小过渡)。 */
  fast: 150,
  /** 200ms —— 基准(hover/颜色淡入,默认属性变化)。 */
  base: 200,
  /** 320ms —— 页面级切换(淡+移)。 */
  page: 320,
  /** 400ms —— 慢(入场沉降,PCL 默认时长)。 */
  slow: 400,
  /** 700ms —— 弹簧/漂移(按钮二段沉降)。 */
  spring: 700,
} as const;

/** 时长键名联合类型。 */
export type DurKey = keyof typeof DUR;

/**
 * 命名缓动(cubic-bezier 字符串,供 WAAPI element.animate() 与 CSS transition)。
 * 镜像 --mo-ease-*。overshoot/elastic 等 cubic-bezier 表达不了的曲线见 easings.ts。
 */
export const EASE = {
  /** 通用属性变化(Material standard)。 */
  standard: "cubic-bezier(.4, 0, .2, 1)",
  /** = Fluent p3,PCL 默认手感基线。 */
  out: "cubic-bezier(.22, 1, .36, 1)",
  /** 强 ease-out,入场/沉降。 */
  emphasized: "cubic-bezier(.16, 1, .3, 1)",
  /** 退场(加速离场)。 */
  accel: "cubic-bezier(.4, 0, 1, 1)",
  /** 回弹(轻过冲)。 */
  back: "cubic-bezier(.34, 1.56, .64, 1)",
} as const;

/** 缓动键名联合类型。 */
export type EaseKey = keyof typeof EASE;

/** 列表错峰步进(毫秒)。镜像 --mo-stagger。 */
export const STAGGER = 28;
