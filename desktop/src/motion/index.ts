/* ============================================================================
 * motion/index.ts —— 动画层统一出口
 *
 * 薄动画层(复现 Classic「手感」),docs/modules/ui-animation.md。CSS 路径用
 * theme/tokens.css 的 --mo-*;JS 路径从这里 import。
 *
 * 用法:
 *   import { createTween, Presence, Motion, motion, DUR, EASE } from "../motion";
 *   import "../motion";  // 仅为让 use:motion 指令的类型增强生效(若只用指令)
 * ========================================================================== */

// 令牌(与 tokens.css 同值)
export { DUR, EASE, STAGGER } from "./tokens";
export type { DurKey, EaseKey } from "./tokens";

// 缓动
export {
  clampEnds,
  linear,
  easeOutFluent,
  easeOutBack,
  easeOutElastic,
  easeOutBounce,
  toEaseIn,
  toEaseInOut,
  EASINGS,
} from "./easings";
export type { EasingFn } from "./easings";

// 插值器
export {
  lerpNumber,
  lerpTuple,
  lerpColor,
  parseColor,
  formatRgba,
} from "./interpolators";
export type { Rgba, ColorSpace, LerpColorOptions } from "./interpolators";

// 目标
export {
  DomStyleTarget,
  CssVarTarget,
  SignalTarget,
  nullTarget,
} from "./targets";
export type { AnimTarget } from "./targets";

// reduced-motion
export { reduced, motionScale, setMotionScale, effectiveScale } from "./reduced";

// 引擎
export {
  animate,
  cancelKey,
  cancelAll,
  activeCount,
  parallel,
  sequence,
} from "./engine";
export type {
  AnimationHandle,
  AnimationStatus,
  AnimateOptions,
  Step,
} from "./engine";

// 预设
export { PRESETS, staggerDelay } from "./presets";
export type { MotionPreset, PresetName, PresetPhase } from "./presets";

// SolidJS 面向用法(含 use:motion 指令的类型增强)
export {
  createTween,
  motion,
  Presence,
  Motion,
  stagger,
  flip,
  measureFlip,
  playFlip,
} from "./solid";
export type {
  TweenToOptions,
  MotionDirectiveValue,
  PresenceProps,
  MotionProps,
  FlipOptions,
} from "./solid";
