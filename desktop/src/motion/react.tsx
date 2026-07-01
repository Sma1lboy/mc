/* ============================================================================
 * motion/react.tsx —— React 动画层(替换 solid.tsx 的公开面)
 *
 * 迁移后整个 app 只有 Toast 消费旧 motion 面(use:motion 入场 + <Presence> 退场),
 * createTween / <Motion> / DUR·EASE(JS)均无 app 消费者。故此层只需两件东西:
 *   - useEntrance(preset)  —— 替换 use:motion:ref 回调,挂载即跑该预设的 enter。
 *   - <Presence>           —— 替换旧 <Presence>:show 变 false 时先跑 exit 再卸载。
 * 两者都直接复用 presets.ts 的 WAAPI 关键帧(与 Classic「手感」零漂移)。
 *
 * 另把 framer 的 motion / AnimatePresence 原样再导出:新写的复杂动画(布局/手势/
 * 编排)请用它们这套 React 惯用法;想对齐 Classic 时长/曲线用 presetTransition()。
 * reduced-motion:两条入口都查 prefers-reduced-motion,为真时直接落终态、跳过动画。
 * ========================================================================== */

import { useEffect, useRef, useState, type ReactNode } from "react";
import { motion, AnimatePresence } from "motion/react";
import { DUR, EASE } from "./tokens";
import { PRESETS, staggerDelay, type PresetName } from "./presets";

export { motion, AnimatePresence };
export { DUR, EASE } from "./tokens";
export { PRESETS, staggerDelay, type PresetName } from "./presets";

/** 是否偏好减少动画(读一次即可,无需响应式)。 */
function prefersReduced(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** 把预设某段(enter/exit)的终态静态写到元素上(reduced-motion 落终态用)。 */
function applyStaticKeyframe(el: HTMLElement, kf: Keyframe): void {
  for (const [k, v] of Object.entries(kf)) {
    if (k === "offset" || k === "easing" || k === "composite" || v == null) continue;
    const prop = k.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase());
    el.style.setProperty(prop, String(v));
  }
}

/** 在元素上跑某预设的某段(enter/exit)。reduced 时落终态并返回 null(无动画)。 */
function runPhase(
  el: HTMLElement,
  preset: PresetName,
  phase: "enter" | "exit",
  extraDelay = 0,
): Animation | null {
  const def = PRESETS[preset]?.[phase];
  if (!def) return null;
  if (PRESETS[preset].transformOrigin) el.style.transformOrigin = PRESETS[preset].transformOrigin!;
  if (prefersReduced()) {
    applyStaticKeyframe(el, def.keyframes[def.keyframes.length - 1]);
    return null;
  }
  const options: KeyframeAnimationOptions = { ...def.options };
  if (extraDelay > 0) options.delay = (options.delay ?? 0) + extraDelay;
  return el.animate(def.keyframes, options);
}

/** useEntrance 选项。 */
export interface EntranceOptions {
  /** 错峰索引(延迟 = index * step)。 */
  index?: number;
  /** 错峰步进(毫秒)。 */
  step?: number;
  /** 关掉入场(只想要 transform-origin / 交给外层控制)。 */
  enter?: boolean;
}

/**
 * 替换 use:motion 指令。返回一个 ref 回调,挂在任意元素上,挂载时跑该预设的 enter,
 * 卸载时取消未完成的入场。不新增 DOM 节点(与旧指令等价)。
 *
 *   const ref = useEntrance("toast");
 *   return <div ref={ref} className="...">…</div>;
 */
export function useEntrance(
  preset: PresetName,
  opts: EntranceOptions = {},
): (el: HTMLElement | null) => void {
  const animRef = useRef<Animation | null>(null);
  const { index, step, enter = true } = opts;
  return (el: HTMLElement | null) => {
    if (!el) {
      // 卸载:取消未完成入场。
      if (animRef.current && animRef.current.playState !== "finished") animRef.current.cancel();
      animRef.current = null;
      return;
    }
    if (enter === false) {
      if (PRESETS[preset].transformOrigin) el.style.transformOrigin = PRESETS[preset].transformOrigin!;
      return;
    }
    const delay = index !== undefined ? staggerDelay(index, step) : 0;
    animRef.current = runPhase(el, preset, "enter", delay);
  };
}

/** <Entrance>:给一个 div 加入场预设的便捷封装(需要额外包一层 div 时用)。 */
export function Entrance(props: {
  preset: PresetName;
  index?: number;
  className?: string;
  style?: React.CSSProperties;
  children: ReactNode;
}): React.ReactElement {
  const ref = useEntrance(props.preset, { index: props.index });
  return (
    <div ref={ref} className={props.className} style={props.style}>
      {props.children}
    </div>
  );
}

/** <Presence> props。 */
export interface PresenceProps {
  /** 是否在场;false 触发退场动画,播完才卸载 children。 */
  show: boolean;
  children: ReactNode;
  /** 退场用的预设名(在 children 的根元素上跑其 exit 段)。 */
  exitPreset?: PresetName;
  /** 退场播完、children 卸载后回调(供调用方清理,如从 store 删条目)。 */
  onExited?: () => void;
}

/**
 * Presence:show=true 渲染 children;show 变 false 时保留 children、在其根 DOM 上跑
 * exit 预设,播完再卸载并回调 onExited。替换旧 solid.tsx 的 <Presence>(那版从
 * children 是否为空推断进退场;React 用显式 show 布尔更贴合)。
 *
 * 用一个 display:contents 包裹层拿到 children 的根元素(不影响布局)。
 */
export function Presence(props: PresenceProps): React.ReactElement | null {
  const { show, exitPreset, onExited } = props;
  // rendered=当前是否挂着 children(可能比 show 滞后一个退场时长)。
  const [rendered, setRendered] = useState(show);
  const wrapRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (show) {
      setRendered(true);
      return;
    }
    if (!rendered) return; // 本就没挂,无需退场
    const el = wrapRef.current?.firstElementChild as HTMLElement | null;
    const anim = el ? runPhase(el, exitPreset ?? "toast", "exit") : null;
    let cancelled = false;
    const finish = () => {
      if (cancelled) return;
      setRendered(false);
      onExited?.();
    };
    if (anim) anim.finished.then(finish, finish);
    else finish();
    return () => {
      cancelled = true;
    };
    // exitPreset/onExited 在一次退场内稳定;仅 show/rendered 驱动。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [show, rendered]);

  if (!rendered) return null;
  return (
    <div ref={wrapRef} style={{ display: "contents" }}>
      {props.children}
    </div>
  );
}

/** framer transition:对齐某预设某段的时长/缓动(供 motion.* 的 initial/animate/exit)。 */
export function presetTransition(
  preset: PresetName,
  phase: "enter" | "exit" = "enter",
): { duration: number; ease: [number, number, number, number]; delay?: number } {
  const opt = PRESETS[preset]?.[phase]?.options ?? {};
  const durMs = typeof opt.duration === "number" ? opt.duration : DUR.base;
  return {
    duration: durMs / 1000,
    ease: bezier(typeof opt.easing === "string" ? opt.easing : EASE.out),
    ...(typeof opt.delay === "number" ? { delay: opt.delay / 1000 } : {}),
  };
}

/** 解析 "cubic-bezier(x1, y1, x2, y2)" → [x1,y1,x2,y2](framer 的 ease 数组形)。 */
function bezier(ease: string): [number, number, number, number] {
  const m = ease.match(/cubic-bezier\(([^)]+)\)/);
  if (!m) return [0.22, 1, 0.36, 1]; // EASE.out 兜底
  const n = m[1].split(",").map((s) => parseFloat(s.trim()));
  return [n[0], n[1], n[2], n[3]];
}
