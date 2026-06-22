/* ============================================================================
 * motion/presets.ts —— Classic「手感」预设表(从 ModAnimation.vb 提取)
 *
 * 逐条复现 Classic 的标志性动作(时长 + 曲线 + 动什么),docs/modules/ui-animation.md §6。
 * 热路径只动 transform/opacity(Classic 随意动 Margin/Width 因 WPF 合成便宜;Web 会每帧
 * 重排)。transform-origin: center(= Classic RenderTransformOrigin 0.5,0.5)。
 *
 * 每个预设给出 enter / exit 两组 WAAPI Keyframe(供 element.animate()),以毫秒时长 +
 * cubic-bezier 字符串缓动表达(WAAPI 仍离主线程,合成友好)。overshoot 用 EASE.back,
 * 强 ease-out 用 EASE.emphasized。reduced-motion 由调用层(solid.tsx)拦截直接落终态。
 * ========================================================================== */

import { DUR, EASE, STAGGER } from "./tokens";

/** 一段 WAAPI 动画的描述:关键帧 + timing。 */
export interface PresetPhase {
  keyframes: Keyframe[];
  options: KeyframeAnimationOptions;
}

/** 一个预设:入场(enter)与退场(exit)两段,均可选。 */
export interface MotionPreset {
  enter?: PresetPhase;
  exit?: PresetPhase;
  /** transform-origin(默认 center)。 */
  transformOrigin?: string;
}

/** 命中所有预设的键名联合。 */
export type PresetName =
  | "pageEnter"
  | "viewCrossfade"
  | "dialog"
  | "dialogBackdrop"
  | "toast"
  | "listStagger"
  | "hoverLift"
  | "expandCollapse"
  | "errorPop"
  | "layoutSwap";

/**
 * 预设表。值用函数式 EASE/DUR 令牌(与 tokens.css 同源)。WAAPI 的 easing 接受
 * cubic-bezier 字符串(EASE.*),overshoot/elastic 这种 cubic-bezier 表达不了的留给
 * createTween + rAF 引擎,不放进这里的 WAAPI 预设。
 */
export const PRESETS: Record<PresetName, MotionPreset> = {
  // page-enter:scale(.96→1) 居中 + 淡入,轻过冲弹入(用 back 近似 OutBack)。
  pageEnter: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "scale(.96)" },
        { opacity: 1, transform: "scale(1)" },
      ],
      options: { duration: DUR.page, easing: EASE.back, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "scale(1)" },
        { opacity: 0, transform: "scale(.95)" },
      ],
      options: { duration: DUR.fast, easing: EASE.accel, fill: "both" },
    },
    transformOrigin: "center",
  },

  // view-crossfade:rightView(news/versions/log)互切,纯淡 + 极轻位移。
  viewCrossfade: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "translateY(6px)" },
        { opacity: 1, transform: "translateY(0)" },
      ],
      options: { duration: DUR.base, easing: EASE.out, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "translateY(0)" },
        { opacity: 0, transform: "translateY(-4px)" },
      ],
      options: { duration: DUR.fast, easing: EASE.accel, fill: "both" },
    },
  },

  // dialog:卡片 scale(.96→1) + 淡入(遮罩单独用 dialogBackdrop)。
  dialog: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "scale(.96)" },
        { opacity: 1, transform: "scale(1)" },
      ],
      options: { duration: DUR.page, easing: EASE.back, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "scale(1)" },
        { opacity: 0, transform: "scale(.97)" },
      ],
      options: { duration: DUR.fast, easing: EASE.accel, fill: "both" },
    },
    transformOrigin: "center",
  },

  // dialog 遮罩:纯淡。
  dialogBackdrop: {
    enter: {
      keyframes: [{ opacity: 0 }, { opacity: 1 }],
      options: { duration: DUR.base, easing: EASE.standard, fill: "both" },
    },
    exit: {
      keyframes: [{ opacity: 1 }, { opacity: 0 }],
      options: { duration: DUR.fast, easing: EASE.standard, fill: "both" },
    },
  },

  // toast:从左下滑入 + 淡入;退场微缩 + 淡出(替掉 setTimeout 硬凑)。
  toast: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "translateX(-16px) translateY(6px)" },
        { opacity: 1, transform: "translateX(0) translateY(0)" },
      ],
      options: { duration: DUR.base, easing: EASE.out, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "translateX(0) scale(1)" },
        { opacity: 0, transform: "translateX(-16px) scale(.98)" },
      ],
      options: { duration: DUR.base, easing: EASE.accel, fill: "both" },
    },
  },

  // list-stagger:每项 translateX(-25→0) + 淡入,错峰由 --i 控制延迟(在 solid.tsx 注入)。
  listStagger: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "translateX(-25px)" },
        { opacity: 1, transform: "translateX(0)" },
      ],
      options: { duration: DUR.page, easing: EASE.out, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "translateX(0)" },
        { opacity: 0, transform: "translateX(-16px)" },
      ],
      options: { duration: DUR.fast, easing: EASE.accel, fill: "both" },
    },
  },

  // hover-lift:轻微抬起 + scale(入场用,作为出现强调;hover 本身走纯 CSS)。
  hoverLift: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "translateY(8px) scale(.99)" },
        { opacity: 1, transform: "translateY(0) scale(1)" },
      ],
      options: { duration: DUR.base, easing: EASE.out, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "translateY(0)" },
        { opacity: 0, transform: "translateY(6px)" },
      ],
      options: { duration: DUR.fast, easing: EASE.accel, fill: "both" },
    },
  },

  // expand-collapse:高度 0fr↔1fr 由 CSS grid 负责,这里只补内容淡入。
  expandCollapse: {
    enter: {
      keyframes: [{ opacity: 0 }, { opacity: 1 }],
      options: { duration: DUR.base, easing: EASE.out, fill: "both", delay: 60 },
    },
    exit: {
      keyframes: [{ opacity: 1 }, { opacity: 0 }],
      options: { duration: DUR.fast, easing: EASE.accel, fill: "both" },
    },
  },

  // error-pop:红叉 scale(0→1) 过冲弹入(用 back 近似 OutBack)。
  errorPop: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "scale(0)" },
        { opacity: 1, transform: "scale(1)" },
      ],
      options: { duration: DUR.slow, easing: EASE.back, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "scale(1)" },
        { opacity: 0, transform: "scale(.6)" },
      ],
      options: { duration: DUR.fast, easing: EASE.accel, fill: "both" },
    },
    transformOrigin: "center",
  },

  // layout-swap:整壳 modrinth↔classic 交叉淡(慢一点,沉稳)。
  layoutSwap: {
    enter: {
      keyframes: [
        { opacity: 0, transform: "scale(1.01)" },
        { opacity: 1, transform: "scale(1)" },
      ],
      options: { duration: DUR.page, easing: EASE.emphasized, fill: "both" },
    },
    exit: {
      keyframes: [
        { opacity: 1, transform: "scale(1)" },
        { opacity: 0, transform: "scale(.995)" },
      ],
      options: { duration: DUR.base, easing: EASE.accel, fill: "both" },
    },
    transformOrigin: "center",
  },
};

/** 每项错峰延迟(毫秒):index * step,默认 STAGGER。 */
export function staggerDelay(index: number, step: number = STAGGER): number {
  return Math.max(0, index) * step;
}
