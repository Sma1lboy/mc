/* ============================================================================
 * theme/palette.ts —— 感知化主题色生成器(OKLCH → in-gamut sRGB hex)
 *
 * 给一份 ToneProfile(tone.ts)+ 一组 ThemeAccent,发出整套 CSS 自定义属性:
 *   --n-1..8        8 级中性灰(从 L 曲线派生,chroma=0)
 *   --a-1..8        8 级强调色(L/C 锚点经非对称 _AdjustLinear 微调 + hue 染色)
 *   --a-semi        极半透明强调(SemiTransparent;我们之前缺的令牌)
 *   --a-bg0/--a-bg1 强调染色表面(浅强调底,Bg0 实色 / Bg1 半透明)
 *   --n-white / --n-white-semi / --n-white-half  纯白叠层(半透明白)
 *
 * 移植自 PCL-CE `ThemeService._CalculateGrays/_CalculateColors` + `LabColor`
 * (Unicolour 的 OKLCH→ScRGB + OklchChromaReduction gamut 映射)。这里**不引任何
 * 颜色库**:OKLCH→OKLab→线性 sRGB 矩阵→gamma 全部内联实现(小且公共领域),
 * gamut 越界时按 PCL 的 ReduceChroma 策略在 OKLCH 下二分降色度,保证输出 in-gamut。
 *
 * docs/modules/ui-polish.md §1。响应式包装在 theme.ts(createMemo→createEffect)。
 * ========================================================================== */

import type { ThemeAccent, ToneProfile } from "./tone";

/* ----------------------------------------------------------------------------
 * 0) 数值钳取小工具
 * -------------------------------------------------------------------------- */

/** 夹到 [0,1]。 */
function clamp01(v: number): number {
  if (!Number.isFinite(v)) return 0;
  return v < 0 ? 0 : v > 1 ? 1 : v;
}

/** 夹到 [lo,hi]。 */
function clampRange(v: number, lo: number, hi: number): number {
  if (!Number.isFinite(v)) return lo;
  return v < lo ? lo : v > hi ? hi : v;
}

/* ----------------------------------------------------------------------------
 * 1) 非对称线性微调 —— 逐字移植 PCL `_AdjustLinear`
 *    adj>0:把 v 朝 1 拉 adj 比例(提亮/增艳,不溢出 1)
 *    adj<0:把 v 朝 0 拉 |adj| 比例(压暗/去饱和,不溢出 0)
 * -------------------------------------------------------------------------- */

/** PCL `_AdjustLinear(value, adjustment)` 的逐字移植(单旋钮非对称 lerp)。 */
export function adjustLinear(value: number, adjustment: number): number {
  if (adjustment === 0) return value;
  // 与 PCL 一致:先把输入夹到合理范围。
  const v = clamp01(value);
  const adj = clampRange(adjustment, -1, 1);
  // 非对称线性插值。
  return adj > 0 ? v + (1 - v) * adj : v + v * adj;
}

/* ----------------------------------------------------------------------------
 * 2) OKLCH → sRGB(内联,无依赖)
 *    OKLCH(L,C,H) → OKLab(L,a,b) → LMS' → LMS → 线性 sRGB → gamma → 0..255
 *    矩阵来自 Björn Ottosson 的 OKLab 定义(公共领域)。
 * -------------------------------------------------------------------------- */

interface LinearRgb {
  r: number;
  g: number;
  b: number;
}

/** OKLCH(L:0..1, C:0..~0.4, H:度) → OKLab(L, a, b)。 */
function oklchToOklab(L: number, C: number, H: number): [number, number, number] {
  const h = (H * Math.PI) / 180;
  return [L, C * Math.cos(h), C * Math.sin(h)];
}

/** OKLab → 线性 sRGB(未夹取,可能越界 [0,1])。 */
function oklabToLinearRgb(L: number, a: number, b: number): LinearRgb {
  // OKLab → LMS'(非线性)
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.291485548 * b;
  // 立方还原到线性 LMS
  const l = l_ * l_ * l_;
  const m = m_ * m_ * m_;
  const s = s_ * s_ * s_;
  // LMS → 线性 sRGB
  return {
    r: 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    g: -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    b: -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  };
}

/** 线性 sRGB 分量 → gamma 编码 sRGB(0..1)。 */
function linearToGamma(c: number): number {
  const x = clamp01(c);
  return x <= 0.0031308 ? 12.92 * x : 1.055 * Math.pow(x, 1 / 2.4) - 0.055;
}

/** 某个线性 sRGB 是否落在 [0,1]^3(含极小容差)内,即 sRGB gamut 内。 */
function inGamut(rgb: LinearRgb): boolean {
  const eps = 1e-4;
  return (
    rgb.r >= -eps &&
    rgb.r <= 1 + eps &&
    rgb.g >= -eps &&
    rgb.g <= 1 + eps &&
    rgb.b >= -eps &&
    rgb.b <= 1 + eps
  );
}

/**
 * 把一个 OKLCH 颜色映射进 sRGB gamut(= PCL 的 ReduceChroma 策略):保持 L、H 不变,
 * 二分搜索最大可表示的色度 C,使线性 sRGB 落在 [0,1]^3。越界时降色度(而非裁剪),
 * 鲜艳色不至于变脏灰偏色——与 Unicolour `OklchChromaReduction` 行为一致。
 */
function reduceChroma(L: number, C: number, H: number): LinearRgb {
  const full = oklabToLinearRgb(...oklchToOklab(L, C, H));
  if (inGamut(full)) return full;
  // L 极端(近黑/近白)时任何色度都难表示,直接退化为无色度的灰。
  if (L <= 0 || L >= 1) {
    return oklabToLinearRgb(...oklchToOklab(clamp01(L), 0, H));
  }
  let lo = 0;
  let hi = C;
  // 25 次二分足够把色度精度收敛到远小于 8-bit 量化步长。
  for (let i = 0; i < 25; i++) {
    const mid = (lo + hi) / 2;
    const rgb = oklabToLinearRgb(...oklchToOklab(L, mid, H));
    if (inGamut(rgb)) lo = mid;
    else hi = mid;
  }
  return oklabToLinearRgb(...oklchToOklab(L, lo, H));
}

/** 0..1 → 两位十六进制。 */
function toHex2(c: number): string {
  const v = Math.round(clamp01(c) * 255);
  return v.toString(16).padStart(2, "0");
}

/**
 * OKLCH → in-gamut sRGB。alpha=1 时返回 `#rrggbb`;否则返回 `#rrggbbaa`(8 位 hex,
 * 现代 WebView 全支持,与现有 hex 令牌同形)。
 */
export function oklchToHex(L: number, C: number, H: number, alpha = 1): string {
  const lin = reduceChroma(clamp01(L), Math.max(0, C), H);
  const r = toHex2(linearToGamma(lin.r));
  const g = toHex2(linearToGamma(lin.g));
  const b = toHex2(linearToGamma(lin.b));
  if (alpha >= 1) return `#${r}${g}${b}`;
  return `#${r}${g}${b}${toHex2(alpha)}`;
}

/* ----------------------------------------------------------------------------
 * 3) 调色板生成 —— PCL `_CalculateGrays` / `_CalculateColors` 的 Web 落地
 * -------------------------------------------------------------------------- */

/** 一份生成好的调色板:CSS 变量名 → 颜色字符串(hex / hex+alpha)。 */
export type Palette = Record<string, string>;

/**
 * 从 ToneProfile + ThemeAccent 生成完整调色板(--n-* / --a-* / 叠层 / 强调表面)。
 *
 * 中性(--n-*):移植 `_CalculateGrays`,8 个亮度锚 L1..8 在 chroma 0 下取灰。
 * PCL 的 Gray1..8 在两套档里亮度走向相反(浅档暗→亮、深档亮→暗);我们的既有语义恒为
 * 「--n-1=最暗底 → --n-8=最亮前景」,故用 `9 - i` 把 L 列**反向**映射进 --n-i,
 * 让深/浅两档都满足该语义(无需各写一套)。
 *
 * 强调(--a-*):移植 `_CalculateColors`,逐级用非对称 _AdjustLinear 调 L/C 后按 hue
 * 染色。a-1 特例(lightAdj*0.1、chromaAdj*0.25)沿用 PCL,保「最深选中底」稳。
 * 另发 PCL 的 SemiTransparent / Bg0 / Bg1 三个我们之前缺的强调令牌。
 */
export function generatePalette(tone: ToneProfile, accent: ThemeAccent): Palette {
  const { hue, lightAdjust: la, chromaAdjust: ca } = accent;
  const out: Palette = {};

  // ---- 中性 --n-1..8(chroma=0;L 列反向映射以贴合既有语义)----
  const grayL = [tone.L1, tone.L2, tone.L3, tone.L4, tone.L5, tone.L6, tone.L7, tone.L8];
  for (let i = 1; i <= 8; i++) {
    const L = grayL[8 - i]; // --n-1 ← L8 …… --n-8 ← L1
    out[`--n-${i}`] = oklchToHex(L, 0, 0, 1);
  }

  // ---- 纯白/前景叠层(半透明白,供亚克力/分隔线等)----
  out["--n-white"] = oklchToHex(tone.LWhite, 0, 0, 1);
  out["--n-white-semi"] = oklchToHex(tone.LWhite, 0, 0, tone.ASemiWhite);
  out["--n-white-half"] = oklchToHex(tone.LWhite, 0, 0, tone.AHalfWhite);

  // ---- 强调 --a-1..8 ----
  //
  // PCL 的 (Ln,Cn) 锚点对里色度峰在 C2/C3,L 走向随档相反(浅档 L1暗→L8亮,深档 L1亮
  // →L8暗)。但**我们的 --a-* 语义契约是固定的**:a-1=最深选中底 → a-8=最浅区块底
  // (见 tokens.css),与明暗模式无关。故按「最深锚点 → a-1」对齐:浅档 L 已升序,直接
  // 取;深档 L 降序,反向取。这样保留 PCL 的 L/C 配对与「a-1 特例旋钮」落在真正最深的
  // 那一级,又满足我们恒定的 a-1→a-8 由深到浅契约(深/浅档都对味)。
  const ascending = tone.L1 <= tone.L8; // 浅档为真(升序),深档为假(降序)。
  // src[k] = 第 k 深锚点(k=0 最深)对应的源序号(1..8)。
  const srcOrder = ascending
    ? [1, 2, 3, 4, 5, 6, 7, 8]
    : [8, 7, 6, 5, 4, 3, 2, 1];
  const lByIdx = (n: number): number =>
    [tone.L1, tone.L2, tone.L3, tone.L4, tone.L5, tone.L6, tone.L7, tone.L8][n - 1];
  const cByIdx = (n: number): number =>
    [tone.C1, tone.C2, tone.C3, tone.C4, tone.C5, tone.C6, tone.C7, tone.C8][n - 1];

  for (let i = 1; i <= 8; i++) {
    const src = srcOrder[i - 1];
    // a-1 特例:亮度/色度旋钮按 PCL 缩放(0.1 / 0.25),其余级用整旋钮。
    const lAdj = i === 1 ? la * 0.1 : la;
    const cAdj = i === 1 ? ca * 0.25 : ca;
    const L = adjustLinear(lByIdx(src), lAdj);
    const C = adjustLinear(cByIdx(src), cAdj);
    out[`--a-${i}`] = oklchToHex(L, C, hue, 1);
  }

  // ---- 强调附加令牌(PCL SemiTransparent / Bg0 / Bg1)----
  // 与上面同理,按「契约语义」而非源序号取锚点:
  //   SemiTransparent ≈ 最浅那级(a-8 同源锚)的极半透明版(近不可见的强调遮罩)。
  //   Bg0 ≈ 中浅实色表面(取偏浅的 C5 配对);Bg1 ≈ 更浅的半透明 hover 染层(C7)。
  const lightestSrc = srcOrder[7]; // 第 8 深 = 最浅。
  {
    const L = adjustLinear(lByIdx(lightestSrc), la);
    const C = adjustLinear(cByIdx(lightestSrc), ca);
    out["--a-semi"] = oklchToHex(L, C, hue, tone.ASemiTransparent);
  }
  // Bg0:取 C5 配对(中段色度)的实色浅强调表面(区块底)。
  {
    const L = adjustLinear(tone.L5, la);
    const C = adjustLinear(tone.C5, ca);
    out["--a-bg0"] = oklchToHex(L, C, hue, 1);
  }
  // Bg1:取偏浅的 C7 配对、半透明强调表面(hover 浅染层)。
  {
    const L = adjustLinear(tone.L7, la);
    const C = adjustLinear(tone.C7, ca);
    out["--a-bg1"] = oklchToHex(L, C, hue, tone.ASemiWhite);
  }

  return out;
}
