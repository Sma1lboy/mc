/* ============================================================================
 * motion/solid.tsx —— SolidJS 面向用法层
 *
 * 模块级单例,无 Context(对齐 store.ts 约定):createTween / use:motion / <Presence>
 * / <Motion> / stagger / flip 直接 import 即用,不引 MotionProvider
 * (docs/modules/ui-animation.md §7/§8)。
 *
 * 分工:
 *   - createTween : JS 驱动的数字补间(进 JSX 的数字读数、弹簧指示器),走 rAF 引擎。
 *   - use:motion  : 挂载即跑 enter 预设(WAAPI),卸载自动取消。
 *   - <Presence>  : 退场后再卸载(WAAPI .finished 拦截卸载),修掉 Toast 的 setTimeout。
 *   - <Motion>    : <Presence> + use:motion 的便捷封装(一个会进退场的 div)。
 *   - stagger     : 错峰延迟(配合 listStagger 预设的 --i)。
 *   - flip        : FLIP 列表重排(measure→invert→play)。
 *
 * reduced-motion:所有 WAAPI/rAF 入口都查 reduced(),为真时直接落终态、跳过动画(§8)。
 * ========================================================================== */

import {
  createSignal,
  onCleanup,
  onMount,
  createRenderEffect,
  children as resolveChildren,
  type Accessor,
  type JSX,
} from "solid-js";

import { DUR } from "./tokens";
import { type EasingFn, EASINGS } from "./easings";
import { animate, type AnimationHandle } from "./engine";
import { reduced } from "./reduced";
import { PRESETS, staggerDelay, type MotionPreset, type PresetName } from "./presets";

/* ============================================================================
 * createTween —— JS 驱动数字补间进 JSX
 * ========================================================================== */

/** createTween 的 animateTo 选项。 */
export interface TweenToOptions {
  duration?: number;
  delay?: number;
  ease?: EasingFn;
  /** 立即跳到目标值(无动画)。 */
  immediate?: boolean;
}

/**
 * 创建一个可动画化的数字信号。返回 [accessor, animateTo]。
 * animateTo 走 rAF 引擎(可打断:省略 from 时从当前值续起,不跳)。每个 tween 用
 * 一个稳定 key(模块自增),保证对同一个 tween 的连续 animateTo 互相打断而非叠加。
 */
export function createTween(
  initial: number,
): [Accessor<number>, (to: number, opts?: TweenToOptions) => AnimationHandle] {
  const [value, setValue] = createSignal<number>(initial);
  const key = `tween#${nextTweenId++}`;

  const animateTo = (to: number, opts: TweenToOptions = {}): AnimationHandle => {
    if (opts.immediate) {
      setValue(to);
      // 返回一个已完成的占位句柄(走 engine 的 scale==0 路径无副作用地落值)。
      return animate({
        key,
        from: to,
        to,
        duration: 0,
        ease: EASINGS.linear,
        onUpdate: (v) => setValue(v),
      });
    }
    return animate({
      key,
      to,
      duration: opts.duration ?? DUR.base,
      delay: opts.delay,
      ease: opts.ease ?? EASINGS.outFluent,
      onUpdate: (v) => setValue(v),
    });
  };

  return [value, animateTo];
}

let nextTweenId = 0;

/* ============================================================================
 * 预设运行(WAAPI)—— enter / exit
 * ========================================================================== */

/** 在元素上跑某预设的某一段(enter/exit)。reduced 时直接落终态、返回已完成。 */
function runPhase(
  el: HTMLElement,
  preset: MotionPreset,
  phase: "enter" | "exit",
  extraDelay = 0,
): Animation | null {
  const def = preset[phase];
  if (!def) return null;
  if (preset.transformOrigin) el.style.transformOrigin = preset.transformOrigin;

  // reduced-motion:不跑 WAAPI,直接把元素摆到该段的终态。
  if (reduced()) {
    const last = def.keyframes[def.keyframes.length - 1];
    applyStaticKeyframe(el, last);
    return null;
  }

  const options: KeyframeAnimationOptions = { ...def.options };
  if (extraDelay > 0) options.delay = (options.delay ?? 0) + extraDelay;
  return el.animate(def.keyframes, options);
}

/** 把单个关键帧的样式静态写到元素上(reduced-motion 落终态用)。 */
function applyStaticKeyframe(el: HTMLElement, kf: Keyframe): void {
  for (const [k, v] of Object.entries(kf)) {
    if (k === "offset" || k === "easing" || k === "composite") continue;
    if (v == null) continue;
    // transform/opacity 等都是合法 CSS 属性名(驼峰转连字符由 style 接受 setProperty 形式)。
    const prop = k.replace(/[A-Z]/g, (m) => "-" + m.toLowerCase());
    el.style.setProperty(prop, String(v));
  }
}

/* ============================================================================
 * use:motion 指令 —— 挂载即入场,卸载自动取消
 * ========================================================================== */

/** use:motion 的取值:预设名或带覆盖项的对象。 */
export interface MotionDirectiveValue {
  preset: PresetName;
  /** 错峰索引(配合 listStagger:延迟 = index*step)。 */
  index?: number;
  /** 错峰步进(毫秒)。 */
  step?: number;
  /** 是否跑入场(默认 true;某些场景只想要 transform-origin/exit)。 */
  enter?: boolean;
}

/**
 * use:motion 指令:元素挂载时跑预设 enter(可错峰),卸载时取消未完成的入场动画。
 * 注:退场需配合 <Presence>(指令无法拦截自身卸载)。
 */
export function motion(
  el: HTMLElement,
  value: Accessor<MotionDirectiveValue | PresetName>,
): void {
  const v = value();
  const cfg: MotionDirectiveValue =
    typeof v === "string" ? { preset: v } : v;
  const preset = PRESETS[cfg.preset];
  if (!preset) return;

  let anim: Animation | null = null;
  onMount(() => {
    if (cfg.enter === false) {
      if (preset.transformOrigin) el.style.transformOrigin = preset.transformOrigin;
      return;
    }
    const delay = cfg.index !== undefined ? staggerDelay(cfg.index, cfg.step) : 0;
    anim = runPhase(el, preset, "enter", delay);
  });

  onCleanup(() => {
    if (anim && anim.playState !== "finished") anim.cancel();
  });
}

/* ============================================================================
 * <Presence> —— 退场后再卸载(WAAPI .finished 拦截卸载)
 * ========================================================================== */

/** <Presence> props。 */
export interface PresenceProps {
  /** 子内容;为 falsy(如 <Show> 不命中)时触发退场再卸载。 */
  children: JSX.Element;
  /** 退场用的预设名(默认按内容自带 use:motion 的 exit;此处作兜底/覆盖)。 */
  exitPreset?: PresetName;
  /** 退场动画播完、内容卸载后回调(供调用方做后续清理,如从 store 删条目)。 */
  onExited?: () => void;
}

/**
 * Presence:包住一个会出现/消失的子树。当 children 从「有」变「无」时,先在已渲染的
 * DOM 上跑退场动画(WAAPI .finished),播完才真正卸载。修掉每个 <Show>/<Switch> 的
 * 瞬切与 Toast 的 setTimeout(220)(§8)。
 *
 * 用法:把 <Show>/<Switch> 放进来——
 *   <Presence exitPreset="dialog"><Show when={open()}><Dialog/></Show></Presence>
 *
 * 实现:解析 children 为响应式节点列表;用一个本地信号 holding 决定实际渲染什么。
 * incoming 有内容 → 立即渲染并(若元素带 use:motion)入场;incoming 变空 → 保留旧
 * 内容、跑退场、播完清空。
 */
export function Presence(props: PresenceProps): JSX.Element {
  const resolved = resolveChildren(() => props.children);
  // 当前实际挂在 DOM 上的节点(可能比 incoming「滞后」一个退场动画的时长)。
  const [held, setHeld] = createSignal<JSX.Element>(undefined, { equals: false });

  let exiting = false;

  // 监听 incoming 变化。createRenderEffect 在首帧渲染前即跑(避免初始内容晚一帧),
  // 之后每次 incoming 变化重跑(进/退场调度)。
  createRenderEffect(() => {
    const incoming = resolved();
    const hasIncoming = !isEmptyChild(incoming);

    if (hasIncoming) {
      // 新内容到来:取消进行中的退场,直接换上(入场由内容自带 use:motion 负责)。
      exiting = false;
      setHeld(() => incoming);
      return;
    }

    // incoming 变空:对当前 held 的 DOM 跑退场,播完清空。
    const current = held();
    if (isEmptyChild(current) || exiting) {
      if (isEmptyChild(current)) setHeld(() => undefined);
      return;
    }
    exiting = true;
    const el = firstElement(current);
    if (!el) {
      setHeld(() => undefined);
      exiting = false;
      props.onExited?.();
      return;
    }
    playExit(el, props.exitPreset).then(() => {
      // 仅当期间没有新内容插入(exiting 仍为真)才清空。
      if (exiting) {
        setHeld(() => undefined);
        exiting = false;
        props.onExited?.();
      }
    });
  });

  return <>{held()}</>;
}

/** 对一个元素跑退场:优先 exitPreset,reduced 则瞬完。返回 Promise<void>。 */
function playExit(el: HTMLElement, exitPreset?: PresetName): Promise<void> {
  if (reduced()) return Promise.resolve();
  const preset = exitPreset ? PRESETS[exitPreset] : undefined;
  if (preset?.exit) {
    if (preset.transformOrigin) el.style.transformOrigin = preset.transformOrigin;
    const anim = el.animate(preset.exit.keyframes, preset.exit.options);
    return anim.finished.then(
      () => undefined,
      () => undefined, // 取消(如被新内容打断)也视为完成
    );
  }
  // 无显式退场预设:默认淡出 + 微缩(toast 退场的通用兜底)。
  const anim = el.animate(
    [
      { opacity: 1, transform: "scale(1)" },
      { opacity: 0, transform: "scale(.98)" },
    ],
    { duration: DUR.base, easing: "cubic-bezier(.4, 0, 1, 1)", fill: "both" },
  );
  return anim.finished.then(
    () => undefined,
    () => undefined,
  );
}

/** 判断 resolved children 是否「空」(undefined/null/false/空数组)。 */
function isEmptyChild(c: JSX.Element): boolean {
  if (c == null || c === false) return true;
  if (Array.isArray(c)) return c.length === 0 || c.every((x) => isEmptyChild(x));
  return false;
}

/** 从 resolved children 里取第一个 HTMLElement(退场动画挂载点)。 */
function firstElement(c: JSX.Element): HTMLElement | null {
  if (c instanceof HTMLElement) return c;
  if (Array.isArray(c)) {
    for (const x of c) {
      const el = firstElement(x as JSX.Element);
      if (el) return el;
    }
  }
  return null;
}

/* ============================================================================
 * <Motion> —— use:motion + Presence 的便捷封装
 * ========================================================================== */

/** <Motion> props。 */
export interface MotionProps {
  /** 预设名。 */
  preset: PresetName;
  /** 错峰索引。 */
  index?: number;
  /** 透传到 div 的额外 class。 */
  class?: string;
  /** 透传 style。 */
  style?: JSX.CSSProperties | string;
  children: JSX.Element;
}

/**
 * Motion:一个自带 enter 预设的 div(卸载时入场动画自动取消)。配合外层 <Presence>
 * 可获得退场。这是「我只想给这块加上 PCL 入场」的最省事写法。
 */
export function Motion(props: MotionProps): JSX.Element {
  return (
    <div
      use:motion={{ preset: props.preset, index: props.index }}
      class={props.class}
      style={props.style}
    >
      {props.children}
    </div>
  );
}

/* ============================================================================
 * stagger —— 错峰延迟(配合 listStagger)
 * ========================================================================== */

/** 错峰延迟(毫秒):index * step。直接用于 use:motion 的 index 或 inline style。 */
export function stagger(index: number, step?: number): number {
  return staggerDelay(index, step);
}

/* ============================================================================
 * flip —— FLIP 列表重排(First-Last-Invert-Play)
 * ========================================================================== */

/** flip() 选项。 */
export interface FlipOptions {
  /** 时长(毫秒)。 */
  duration?: number;
  /** 缓动 cubic-bezier 字符串。 */
  easing?: string;
  /** 只对带此 data 属性的直接子项做 FLIP(默认所有元素直接子项)。 */
  selector?: string;
}

/** 记录一次测量:每个子元素的位置矩形。 */
type FlipSnapshot = Map<Element, DOMRect>;

/**
 * 测量容器内子项当前位置,返回快照。在「即将改变 DOM 顺序/过滤」之前调用。
 */
export function measureFlip(container: HTMLElement, selector?: string): FlipSnapshot {
  const snap: FlipSnapshot = new Map();
  const kids = selector
    ? container.querySelectorAll<HTMLElement>(selector)
    : container.children;
  for (const el of Array.from(kids)) {
    snap.set(el, (el as HTMLElement).getBoundingClientRect());
  }
  return snap;
}

/**
 * 在 DOM 已更新后,用旧快照做 invert→play:把每个仍存在的子项从「旧位置」补到
 * 「新位置」(WAAPI translate)。reduced 时直接跳过(§8)。
 *
 * 典型用法(SolidJS):在更新列表的信号前 measureFlip,更新后用 queueMicrotask/
 * requestAnimationFrame 调 playFlip。
 */
export function playFlip(
  container: HTMLElement,
  before: FlipSnapshot,
  opts: FlipOptions = {},
): void {
  if (reduced()) return;
  const duration = opts.duration ?? DUR.page;
  const easing = opts.easing ?? "cubic-bezier(.22, 1, .36, 1)";
  const kids = opts.selector
    ? container.querySelectorAll<HTMLElement>(opts.selector)
    : container.children;

  for (const node of Array.from(kids)) {
    const el = node as HTMLElement;
    const prev = before.get(el);
    if (!prev) continue; // 新增项:交给 use:motion 入场,FLIP 不管
    const next = el.getBoundingClientRect();
    const dx = prev.left - next.left;
    const dy = prev.top - next.top;
    if (dx === 0 && dy === 0) continue; // 没动
    el.animate(
      [
        { transform: `translate(${dx}px, ${dy}px)` },
        { transform: "translate(0, 0)" },
      ],
      { duration, easing, fill: "none" },
    );
  }
}

/**
 * flip(container):便捷高阶——返回一个 run(mutate) 包装器,自动在 mutate 前后做
 * measure/play。mutate 内做实际 DOM 顺序变更(或触发会改变布局的信号);本函数在其
 * 后用 requestAnimationFrame 让浏览器先 reflow,再 playFlip。
 */
export function flip(
  container: HTMLElement,
  opts: FlipOptions = {},
): (mutate: () => void) => void {
  return (mutate: () => void): void => {
    const before = measureFlip(container, opts.selector);
    mutate();
    // 等下一帧让 DOM/布局更新后再补差。
    requestAnimationFrame(() => playFlip(container, before, opts));
  };
}

/* ============================================================================
 * SolidJS 模块增强:声明 use:motion 指令的类型
 * ========================================================================== */

declare module "solid-js" {
  namespace JSX {
    interface Directives {
      // use:motion 取值:预设名或带覆盖项的对象。
      motion: MotionDirectiveValue | PresetName;
    }
  }
}

// 注:use:motion 编译后按名字引用导出的 `motion` 函数。消费组件须确保 `motion`
// 被实际触达(如 `void motion;`),否则打包器可能将其摇掉——见 components/Toast.tsx。
