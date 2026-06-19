/* ============================================================================
 * motion/reduced.ts —— prefers-reduced-motion 的 JS 侧真相源
 *
 * tokens.css 的 @media(prefers-reduced-motion) + !important 只能盖 CSS;rAF/WAAPI
 * 动画完全无视它(docs/modules/ui-animation.md §8)。这里读一次 matchMedia + 监听
 * 变化,所有 JS 动画层据此「dur *= reduced()?0:1」并直接落终值。
 *
 * motionScale 是全局动画时长系数(= PCL 的 AniSpeed/Scale):1=正常,0=瞬切。
 * reduced 时强制路由到 0(由 effectiveScale 合成),但保留独立信号以便调试/将来
 * 「降低动画强度」设置项可写。
 * ========================================================================== */

import { createSignal, type Accessor } from "solid-js";

const QUERY = "(prefers-reduced-motion: reduce)";

// SSR / 非浏览器环境兜底(理论上 Tauri webview 总有 window,但保持纯函数安全)。
const mql: MediaQueryList | null =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(QUERY)
    : null;

const [reducedSignal, setReducedSignal] = createSignal<boolean>(mql?.matches ?? false);

// 监听系统偏好变化(用户运行时切换无需重启)。addEventListener 现代;旧 WebKit
// 仅有 addListener,做一次能力探测兜底。
if (mql) {
  const onChange = (e: MediaQueryListEvent): void => {
    setReducedSignal(e.matches);
  };
  if (typeof mql.addEventListener === "function") {
    mql.addEventListener("change", onChange);
  } else if (typeof (mql as MediaQueryList).addListener === "function") {
    // 已废弃 API,仅为老内核兜底。
    (mql as MediaQueryList).addListener(onChange);
  }
}

/** 系统是否要求减弱动画。响应式 accessor(信号),组件/effect 内读会自动追踪。 */
export const reduced: Accessor<boolean> = reducedSignal;

/**
 * 全局动画时长系数(= PCL AniSpeed)。1=正常。用户可写(将来设置项),但实际生效
 * 值还要与 reduced 合成,见 effectiveScale()。
 */
export const [motionScale, setMotionScale] = createSignal<number>(1);

/**
 * 实际生效的时长系数:reduced 时强制 0(瞬切),否则取 motionScale 并钳到 [0,∞)。
 * engine 用它换算每个 tween 的有效时长。
 */
export function effectiveScale(): number {
  if (reduced()) return 0;
  const s = motionScale();
  return s > 0 ? s : 0;
}
