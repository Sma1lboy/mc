/* ============================================================================
 * motion/interpolators.ts —— 类型化插值器(IValueProcessor 的 Web 等价)
 *
 * DOM 属性动画交给浏览器插值;只有「JS 驱动值」(数字读数、自定义属性、信号、
 * 颜色补间)才需要在 TS 里 lerp(docs/modules/ui-animation.md §5/§7)。
 *
 * 颜色默认 sRGB 分量线性 lerp —— 对齐 PCL `NColor`(0..255 RGBA 的 Vector4
 * 线性插值,非感知)。{ space: 'oklch' } 作可选感知升级:OKLCH→sRGB 数学内联
 * (公版,无第三方依赖)。
 * ========================================================================== */

/** 数字线性插值。 */
export function lerpNumber(from: number, to: number, t: number): number {
  return from + (to - from) * t;
}

/** 等长数字元组逐分量插值(transform 矩阵/厚度等)。 */
export function lerpTuple<T extends readonly number[]>(from: T, to: T, t: number): number[] {
  const n = Math.min(from.length, to.length);
  const out = new Array<number>(n);
  for (let i = 0; i < n; i++) out[i] = from[i] + (to[i] - from[i]) * t;
  return out;
}

/** RGBA 颜色(分量 0..255,alpha 0..1)。 */
export interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

/** 颜色插值色彩空间:sRGB(默认,对齐 PCL)或 OKLCH(感知,可选)。 */
export type ColorSpace = "srgb" | "oklch";

/** lerpColor 选项。 */
export interface LerpColorOptions {
  space?: ColorSpace;
}

const clamp01 = (v: number): number => (v < 0 ? 0 : v > 1 ? 1 : v);
const clamp255 = (v: number): number => (v < 0 ? 0 : v > 255 ? 255 : v);

/**
 * 解析 CSS 颜色字符串为 Rgba。支持 #rgb / #rgba / #rrggbb / #rrggbbaa、
 * rgb()/rgba()。解析失败回落透明黑(不抛错,动画层须容错)。
 */
export function parseColor(input: string): Rgba {
  const s = input.trim();
  if (s.startsWith("#")) {
    const hex = s.slice(1);
    const expand = (h: string): number => parseInt(h + h, 16);
    if (hex.length === 3) {
      return { r: expand(hex[0]), g: expand(hex[1]), b: expand(hex[2]), a: 1 };
    }
    if (hex.length === 4) {
      return {
        r: expand(hex[0]),
        g: expand(hex[1]),
        b: expand(hex[2]),
        a: expand(hex[3]) / 255,
      };
    }
    if (hex.length === 6) {
      return {
        r: parseInt(hex.slice(0, 2), 16),
        g: parseInt(hex.slice(2, 4), 16),
        b: parseInt(hex.slice(4, 6), 16),
        a: 1,
      };
    }
    if (hex.length === 8) {
      return {
        r: parseInt(hex.slice(0, 2), 16),
        g: parseInt(hex.slice(2, 4), 16),
        b: parseInt(hex.slice(4, 6), 16),
        a: parseInt(hex.slice(6, 8), 16) / 255,
      };
    }
  }
  const m = s.match(/rgba?\(([^)]+)\)/i);
  if (m) {
    const parts = m[1].split(/[,\s/]+/).filter(Boolean);
    const r = parseFloat(parts[0] ?? "0");
    const g = parseFloat(parts[1] ?? "0");
    const b = parseFloat(parts[2] ?? "0");
    const a = parts.length > 3 ? parseFloat(parts[3]) : 1;
    return { r, g, b, a: Number.isFinite(a) ? a : 1 };
  }
  return { r: 0, g: 0, b: 0, a: 0 };
}

/** 把 Rgba 格式化为 rgba() 字符串(分量取整,alpha 三位小数)。 */
export function formatRgba(c: Rgba): string {
  const r = Math.round(clamp255(c.r));
  const g = Math.round(clamp255(c.g));
  const b = Math.round(clamp255(c.b));
  const a = Math.round(clamp01(c.a) * 1000) / 1000;
  return `rgba(${r}, ${g}, ${b}, ${a})`;
}

/* ---- OKLCH ↔ sRGB(可选感知插值,公版数学,内联无依赖)---- */

// sRGB 传输函数(gamma)。
const srgbToLinear = (c: number): number => {
  const x = c / 255;
  return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
};
const linearToSrgb = (c: number): number => {
  const x = c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
  return clamp255(x * 255);
};

/** 线性 sRGB → OKLab(Björn Ottosson 矩阵)。 */
function linearSrgbToOklab(r: number, g: number, b: number): [number, number, number] {
  const l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
  const m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
  const s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
  const l_ = Math.cbrt(l);
  const m_ = Math.cbrt(m);
  const s_ = Math.cbrt(s);
  return [
    0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_,
    1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_,
    0.0259040371 * l_ + 0.7827717662 * m_ - 0.808675766 * s_,
  ];
}

/** OKLab → 线性 sRGB(逆矩阵)。 */
function oklabToLinearSrgb(L: number, a: number, b: number): [number, number, number] {
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.291485548 * b;
  const l = l_ ** 3;
  const m = m_ ** 3;
  const s = s_ ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

/** Rgba(0..255) → OKLCH 极坐标(L, C, H弧度, alpha)。 */
function rgbaToOklch(c: Rgba): [number, number, number, number] {
  const [L, a, b] = linearSrgbToOklab(
    srgbToLinear(c.r),
    srgbToLinear(c.g),
    srgbToLinear(c.b),
  );
  const C = Math.hypot(a, b);
  const H = Math.atan2(b, a);
  return [L, C, H, c.a];
}

/** OKLCH 极坐标 → Rgba(0..255)。 */
function oklchToRgba(L: number, C: number, H: number, alpha: number): Rgba {
  const a = C * Math.cos(H);
  const b = C * Math.sin(H);
  const [lr, lg, lb] = oklabToLinearSrgb(L, a, b);
  return {
    r: linearToSrgb(lr),
    g: linearToSrgb(lg),
    b: linearToSrgb(lb),
    a: alpha,
  };
}

/** 取最短弧的角度插值(避免 OKLCH 跨色相绕远路)。 */
function lerpAngle(a: number, b: number, t: number): number {
  let d = b - a;
  while (d > Math.PI) d -= 2 * Math.PI;
  while (d < -Math.PI) d += 2 * Math.PI;
  return a + d * t;
}

/**
 * 颜色插值。默认 sRGB 分量线性(对齐 PCL NColor 观感);space:'oklch' 走感知
 * 插值(色相走最短弧)。输入/输出均为 CSS 颜色字符串。
 */
export function lerpColor(
  from: string,
  to: string,
  t: number,
  opts: LerpColorOptions = {},
): string {
  const a = parseColor(from);
  const b = parseColor(to);
  if (opts.space === "oklch") {
    const [L1, C1, H1, A1] = rgbaToOklch(a);
    const [L2, C2, H2, A2] = rgbaToOklch(b);
    const out = oklchToRgba(
      L1 + (L2 - L1) * t,
      C1 + (C2 - C1) * t,
      lerpAngle(H1, H2, t),
      A1 + (A2 - A1) * t,
    );
    return formatRgba(out);
  }
  // 默认 sRGB 分量线性。
  return formatRgba({
    r: a.r + (b.r - a.r) * t,
    g: a.g + (b.g - a.g) * t,
    b: a.b + (b.b - a.b) * t,
    a: a.a + (b.a - a.a) * t,
  });
}
