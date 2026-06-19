/* ============================================================================
 * motion/engine.ts —— 一条 rAF ticker 驱动的值补间引擎
 *
 * PCL AnimationService 的 Web 等价,但删掉多线程 Channel/worker(Web 单 UI 线程,
 * rAF 已在其上)。只为 CSS/WAAPI 做不到的 ~10% 服务:可打断/从当前值重定向的值
 * 补间、弹簧/拖拽跟随、overshoot/elastic 缓动(docs/modules/ui-animation.md §3/§7)。
 *
 * 设计要点:
 *   - 一个模块级单例 Set<Tween> + 一条 rAF;集合空了就停 rAF(空闲零 CPU)。
 *   - keyed-cancel(Map<key, handle>):同 key 再启动会取消旧 tween;若新 tween 省略
 *     from,则捕获「当前值」作 from(= PCL 命脉:快速来回不跳)。
 *   - honor reduced(): effectiveScale()==0 时直接落终值、不进 rAF。
 *   - 基于时间的进度(elapsed/dur),不依赖全局 Fps(掉帧时 PCL 帧计数会失真)。
 * ========================================================================== */

import { effectiveScale } from "./reduced";
import { type EasingFn } from "./easings";

/** 动画状态,对齐 WAAPI playState 语义。 */
export type AnimationStatus = "running" | "finished" | "cancelled";

/** animate() 的返回句柄。 */
export interface AnimationHandle {
  /** 当前状态(running→finished/cancelled,单向)。 */
  readonly status: AnimationStatus;
  /** 完成时 resolve、取消时 reject(reason='cancelled')的 Promise。 */
  readonly finished: Promise<void>;
  /** 取消动画(停在当前值,不落终值)。幂等。 */
  cancel(): void;
}

/** animate() 选项(数字补间)。 */
export interface AnimateOptions {
  /** keyed-cancel 键:同 key 再启动取消旧的(PCL 命名冲突取消)。 */
  key?: string;
  /** 起始值;省略 = 启动时读「当前值」(同 key 时为被取消 tween 的当前值)。 */
  from?: number;
  /** 目标值。 */
  to: number;
  /** 时长(毫秒,未经 motionScale)。 */
  duration: number;
  /** 起始延迟(毫秒)。 */
  delay?: number;
  /** 缓动纯函数。 */
  ease: EasingFn;
  /** 每帧回调,写信号 / style / 自定义属性(= IAnimatable.set)。 */
  onUpdate: (v: number) => void;
}

/** 内部 tween 记录。 */
interface Tween {
  key?: string;
  from: number;
  to: number;
  /** 已套用 motionScale 的有效时长。 */
  duration: number;
  delay: number;
  ease: EasingFn;
  onUpdate: (v: number) => void;
  /** 首帧时间戳(用于 elapsed);-1 = 尚未进入第一帧。 */
  startTime: number;
  /** 最近一次写出的值(供 keyed-cancel 捕获当前值作新 from)。 */
  current: number;
  handle: TweenHandle;
}

/** 运行中的 tween 集合。 */
const running = new Set<Tween>();
/** keyed tween 索引(同 key 再启动取消旧的)。 */
const keyed = new Map<string, Tween>();
/** 当前 rAF id;0 = 未运行。 */
let rafId = 0;

/** 句柄实现:状态机 + resolve/reject 闭包。 */
class TweenHandle implements AnimationHandle {
  status: AnimationStatus = "running";
  finished: Promise<void>;
  private resolveFn!: () => void;
  private rejectFn!: (reason: unknown) => void;
  /** 关联的 tween,取消时用于从集合摘除。 */
  tween: Tween | null = null;

  constructor() {
    this.finished = new Promise<void>((resolve, reject) => {
      this.resolveFn = resolve;
      // 取消导致的 reject 在外部已被 catch 的语义下不应造成 unhandledrejection;
      // 调用方若不关心取消,可忽略 finished。这里仍 reject 以便 sequence 短路。
      this.rejectFn = reject;
    });
    // 避免取消时无监听者触发 unhandledrejection 噪音。
    this.finished.catch(() => {
      /* swallow:取消是正常控制流,非错误 */
    });
  }

  resolve(): void {
    if (this.status !== "running") return;
    this.status = "finished";
    this.resolveFn();
  }

  cancel(): void {
    if (this.status !== "running") return;
    this.status = "cancelled";
    const t = this.tween;
    if (t) {
      running.delete(t);
      if (t.key !== undefined && keyed.get(t.key) === t) keyed.delete(t.key);
      this.tween = null;
    }
    this.rejectFn("cancelled");
    maybeStopLoop();
  }
}

/** rAF 主循环:推进每个 tween,完成的摘除并 resolve;集合空了停 rAF。 */
function tick(now: number): void {
  // 快照避免迭代中增删(onUpdate 可能再触发 animate)。
  const snapshot = Array.from(running);
  for (const t of snapshot) {
    if (!running.has(t)) continue; // 本帧内被取消
    if (t.startTime < 0) t.startTime = now;
    const elapsed = now - t.startTime - t.delay;
    if (elapsed < 0) continue; // 仍在 delay 中

    if (t.duration <= 0) {
      // 零时长(或 scale 后趋零):直接落终值。
      t.current = t.to;
      t.onUpdate(t.to);
      finishTween(t);
      continue;
    }

    const p = elapsed >= t.duration ? 1 : elapsed / t.duration;
    const v = t.from + (t.to - t.from) * t.ease(p);
    t.current = v;
    t.onUpdate(v);

    if (p >= 1) finishTween(t);
  }

  if (running.size > 0) {
    rafId = requestAnimationFrame(tick);
  } else {
    rafId = 0;
  }
}

/** 完成一个 tween:摘除 + 清 keyed + resolve 句柄。 */
function finishTween(t: Tween): void {
  running.delete(t);
  if (t.key !== undefined && keyed.get(t.key) === t) keyed.delete(t.key);
  t.handle.resolve();
}

/** 集合非空且 rAF 未运行 → 启动。 */
function ensureLoop(): void {
  if (rafId === 0 && running.size > 0) {
    rafId = requestAnimationFrame(tick);
  }
}

/** 集合空了 → 停 rAF(空闲零 CPU)。 */
function maybeStopLoop(): void {
  if (running.size === 0 && rafId !== 0) {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }
}

/**
 * 启动一个数字补间。返回句柄(status/finished/cancel)。
 *
 * - 同 key:取消旧 tween;若新 opts 省略 from,捕获旧 tween 的当前值作 from
 *   (没有旧 tween 则 from 默认 to,即「已在终点」无动画感)。
 * - reduced:effectiveScale()==0 → 直接同步落终值、返回已 finished 的句柄。
 */
export function animate(opts: AnimateOptions): AnimationHandle {
  const scale = effectiveScale();

  // reduced / scale==0:不进 rAF,同步落终值。
  if (scale === 0) {
    if (opts.key !== undefined) {
      const prev = keyed.get(opts.key);
      if (prev) prev.handle.cancel();
    }
    opts.onUpdate(opts.to);
    const h = new TweenHandle();
    h.resolve();
    return h;
  }

  let from = opts.from;
  // keyed-cancel:取消同 key 旧 tween,捕获当前值作 from(若省略)。
  if (opts.key !== undefined) {
    const prev = keyed.get(opts.key);
    if (prev) {
      if (from === undefined) from = prev.current;
      prev.handle.cancel();
    }
  }
  if (from === undefined) from = opts.to;

  const handle = new TweenHandle();
  const tween: Tween = {
    key: opts.key,
    from,
    to: opts.to,
    duration: Math.max(0, opts.duration) * scale,
    delay: Math.max(0, opts.delay ?? 0) * scale,
    ease: opts.ease,
    onUpdate: opts.onUpdate,
    startTime: -1,
    current: from,
    handle,
  };
  handle.tween = tween;

  running.add(tween);
  if (opts.key !== undefined) keyed.set(opts.key, tween);
  ensureLoop();
  return handle;
}

/** 取消某个 keyed 动画(若存在)。供外部按 key 主动打断。 */
export function cancelKey(key: string): void {
  const t = keyed.get(key);
  if (t) t.handle.cancel();
}

/** 取消所有运行中的动画(如布局整壳卸载时)。 */
export function cancelAll(): void {
  for (const t of Array.from(running)) t.handle.cancel();
}

/** 当前运行中的 tween 数(调试/测试用)。 */
export function activeCount(): number {
  return running.size;
}

/* ---- 组合子(Parallel / Sequential)---- */

/** 一个可被 sequence/parallel 编排的步骤:返回一个带 finished 的句柄。 */
export interface Step {
  finished: Promise<void>;
  cancel(): void;
}

/** 并行:全部启动,等全部 finished(任一取消则整体视为取消)。 */
export function parallel(...steps: Step[]): Step {
  let cancelled = false;
  const cancel = (): void => {
    if (cancelled) return;
    cancelled = true;
    for (const s of steps) s.cancel();
  };
  const finished = Promise.all(steps.map((s) => s.finished)).then(() => undefined);
  return { finished, cancel };
}

/**
 * 串行:按序 await 每步的 finished;任一步取消(reject)则短路并取消后续。
 * step 可以是 Step,也可以是返回 Step 的工厂(惰性,前一步完成后才启动下一步)。
 */
export function sequence(...steps: (Step | (() => Step))[]): Step {
  let cancelled = false;
  let activeCancel: (() => void) | null = null;

  const cancel = (): void => {
    if (cancelled) return;
    cancelled = true;
    if (activeCancel) activeCancel();
  };

  const finished = (async (): Promise<void> => {
    for (const stepOrFactory of steps) {
      if (cancelled) throw "cancelled";
      const step = typeof stepOrFactory === "function" ? stepOrFactory() : stepOrFactory;
      activeCancel = step.cancel;
      await step.finished; // 取消时这里 reject 'cancelled',向上抛出
    }
  })();
  // 吞掉取消导致的 reject 噪音(取消是正常控制流)。
  finished.catch(() => {
    /* swallow */
  });

  return { finished, cancel };
}
