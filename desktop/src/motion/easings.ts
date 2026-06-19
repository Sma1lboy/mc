/* ============================================================================
 * motion/easings.ts —— 纯函数缓动库(逐字移植 PCL-CE)
 *
 * 「忠实即手感」:这些不是教科书 Penner 版,而是 PCL `ModAnimation.vb` /
 * PCL-CE `IEasing` 的非标准 Back/Elastic 公式(docs/modules/ui-animation.md §5)。
 * 用 Penner 标准曲线替换会丢掉 PCL 的招牌过冲/振荡观感,故照搬。
 *
 * 约定:所有缓动 (t:number)=>number,t∈[0,1],返回插值进度。多数为 ease-out
 * (PCL 几乎一切默认 OutFluent)。端点用 clampEnds 钳到精确 0/1,避免浮点残差。
 *
 * 哪些进 CSS / 哪些只能 JS(§5):多项式/正弦/指数/圆 → cubic-bezier(EASE);
 * Back ≈ EASE.back;Elastic/Bounce/Composite/Spring 只能在此 JS 采样(或预采样
 * 成 WAAPI linear())。
 * ========================================================================== */

/** 缓动函数签名:进度 t∈[0,1] → 缓动后进度。 */
export type EasingFn = (t: number) => number;

/**
 * 端点钳:把 t<=0 / t>=1 钳到精确 0/1,中间区间走 f。
 * 既保证起止值精确命中(避免浮点尾差),又让越界 t 表现良好。
 */
export const clampEnds =
  (f: EasingFn): EasingFn =>
  (t: number): number =>
    t <= 0 ? 0 : t >= 1 ? 1 : f(t);

/** 线性(无缓动)。 */
export const linear: EasingFn = (t) => t;

/**
 * Fluent ease-out(幂曲线):PCL 默认手感基线,p 越大收尾越软。
 * = 1 - (1 - t)^p。p=3 对应 CSS cubic-bezier(.22,1,.36,1)。
 */
export const easeOutFluent = (t: number, p = 3): number => 1 - (1 - t) ** p;

/**
 * Back ease-out(回弹):cos 项给末段过冲,p 控制幂次。
 * = 1 - (1 - t)^p * cos(1.5π t)。这是 PCL 的非标准 Back(招牌轻弹)。
 */
export const easeOutBack = (t: number, p = 2): number =>
  1 - (1 - t) ** p * Math.cos(1.5 * Math.PI * t);

/**
 * Elastic ease-out(弹性,衰减正弦):PCL 的非标准 Elastic,末段多次振荡收敛。
 * u=1-t;= 1 - u^((p-1)*0.25) * cos((p-3.5)π (1-u)^1.5)。
 */
export const easeOutElastic = (t: number, p = 2): number => {
  const u = 1 - t;
  return 1 - u ** ((p - 1) * 0.25) * Math.cos((p - 3.5) * Math.PI * (1 - u) ** 1.5);
};

/**
 * Bounce ease-out(回弹落地):标准四段二次曲线(n1=7.5625, d1=2.75)。
 * PCL 的 loading/error 落地感来源之一。
 */
export const easeOutBounce = (t: number): number => {
  const n1 = 7.5625;
  const d1 = 2.75;
  if (t < 1 / d1) {
    return n1 * t * t;
  } else if (t < 2 / d1) {
    const x = t - 1.5 / d1;
    return n1 * x * x + 0.75;
  } else if (t < 2.5 / d1) {
    const x = t - 2.25 / d1;
    return n1 * x * x + 0.9375;
  } else {
    const x = t - 2.625 / d1;
    return n1 * x * x + 0.984375;
  }
};

/* ---- 由 ease-out 翻折出 in / inOut 变体(PCL EasePower 也是同源翻折)---- */

/** 把一条 ease-out 缓动翻折成 ease-in(time/value 双反)。 */
export const toEaseIn =
  (out: EasingFn): EasingFn =>
  (t: number): number =>
    1 - out(1 - t);

/** 把一条 ease-out 缓动拼成 ease-inOut(前半 in、后半 out)。 */
export const toEaseInOut =
  (out: EasingFn): EasingFn =>
  (t: number): number =>
    t < 0.5 ? (1 - out(1 - 2 * t)) / 2 : (1 + out(2 * t - 1)) / 2;

/**
 * EASINGS —— 命名缓动注册表(对齐 PCL EasingConverter 常用项)。
 * 全部已套 clampEnds,可直接喂给 engine.animate({ ease })。带参数的幂曲线
 * 用 PCL 的 Weak2/Middle3/Strong4/ExtraStrong5 指数档(EasePower 枚举)预绑。
 */
export const EASINGS: Record<string, EasingFn> = {
  linear: clampEnds(linear),

  // Fluent(幂)ease-out,PCL 的「一切默认」。
  outFluent: clampEnds((t) => easeOutFluent(t, 3)),
  outFluentWeak: clampEnds((t) => easeOutFluent(t, 2)),
  outFluentMiddle: clampEnds((t) => easeOutFluent(t, 3)),
  outFluentStrong: clampEnds((t) => easeOutFluent(t, 4)),
  outFluentExtraStrong: clampEnds((t) => easeOutFluent(t, 5)),
  inFluent: clampEnds(toEaseIn((t) => easeOutFluent(t, 3))),
  inFluentWeak: clampEnds(toEaseIn((t) => easeOutFluent(t, 2))),
  inOutFluent: clampEnds(toEaseInOut((t) => easeOutFluent(t, 3))),

  // Back(回弹)。
  outBack: clampEnds((t) => easeOutBack(t, 2)),
  outBackWeak: clampEnds((t) => easeOutBack(t, 1.5)),
  inBack: clampEnds(toEaseIn((t) => easeOutBack(t, 2))),
  inOutBack: clampEnds(toEaseInOut((t) => easeOutBack(t, 2))),

  // Elastic(弹性)。
  outElastic: clampEnds((t) => easeOutElastic(t, 2)),
  inElastic: clampEnds(toEaseIn((t) => easeOutElastic(t, 2))),

  // Bounce(落地回弹)。
  outBounce: clampEnds(easeOutBounce),
  inBounce: clampEnds(toEaseIn(easeOutBounce)),
};
