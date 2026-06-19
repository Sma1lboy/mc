/* ============================================================================
 * motion/targets.ts —— 动画目标抽象(PCL IAnimatable 的 Web 等价)
 *
 * AnimTarget<T> 把「读当前值 / 写新值」收成一对函数,让 engine 不关心值最终落到
 * DOM style、CSS 自定义属性、还是 SolidJS 信号(docs/modules/ui-animation.md §7)。
 *   - DomStyleTarget : 直接写 element.style[prop](数字 + 单位)
 *   - CssVarTarget   : 写自定义属性 --x(配合 CSS 消费,合成线程友好)
 *   - SignalTarget   : 写 SolidJS 信号(JSX 里读)
 *   - nullTarget     : 黑洞(reduced-motion 时路由到此,只落终值不动画)
 * ========================================================================== */

/** 动画目标:读/写一个值。engine 每帧 set(lerp(...))。 */
export interface AnimTarget<T = number> {
  get(): T;
  set(v: T): void;
}

/**
 * DOM style 数字属性目标。写时拼上单位(默认 'px';opacity/scale 等无单位传 '')。
 * 读时解析 computed/inline 数字(解析失败回落 0)。
 */
export function DomStyleTarget(
  el: HTMLElement,
  prop: string,
  unit = "px",
): AnimTarget<number> {
  return {
    get(): number {
      const inline = el.style.getPropertyValue(prop);
      const raw = inline !== "" ? inline : getComputedStyle(el).getPropertyValue(prop);
      const n = parseFloat(raw);
      return Number.isFinite(n) ? n : 0;
    },
    set(v: number): void {
      el.style.setProperty(prop, `${v}${unit}`);
    },
  };
}

/**
 * CSS 自定义属性目标(--x)。值按数字写;若需单位用 format 回调包裹。
 * 自定义属性变化不触发样式重算的布局阶段(交给 CSS 消费方决定),热路径首选。
 */
export function CssVarTarget(
  el: HTMLElement,
  name: `--${string}`,
  format: (v: number) => string = (v) => String(v),
): AnimTarget<number> {
  return {
    get(): number {
      const raw =
        el.style.getPropertyValue(name) ||
        getComputedStyle(el).getPropertyValue(name);
      const n = parseFloat(raw);
      return Number.isFinite(n) ? n : 0;
    },
    set(v: number): void {
      el.style.setProperty(name, format(v));
    },
  };
}

/** SolidJS 信号目标:用 accessor 读、setter 写。 */
export function SignalTarget<T>(get: () => T, set: (v: T) => void): AnimTarget<T> {
  return { get, set };
}

/**
 * 黑洞目标:get 恒返回 0、set 丢弃。reduced-motion 时把动画路由到此(配合直接
 * 落终值),保证 rAF/WAAPI 不违背系统减弱动画偏好(§8)。
 */
export const nullTarget: AnimTarget<number> = {
  get(): number {
    return 0;
  },
  set(_v: number): void {
    /* 丢弃 */
  },
};
