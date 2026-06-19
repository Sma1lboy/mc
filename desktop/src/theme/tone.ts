/* ============================================================================
 * theme/tone.ts —— 感知化主题色引擎的「锚点表」(ToneProfile)
 *
 * 1:1 移植 PCL-CE `PCL.Core/UI/Theme/ToneProfile.cs` + `ToneProfileConfig.cs`:
 * 每个明暗模式各一份 OKLCH 的 L/C/alpha 锚点。生成器(palette.ts)拿这份锚点 +
 * 一组 ThemeAccent 三元组,用 OKLCH 派生 8 级中性(--n-*)与 8 级强调(--a-*)。
 *
 * 与现有朴素 HSL theme.ts 的关系:严格升级——HSL 手调偏移数组被这份感知锚点表 +
 * 非对称 _AdjustLinear 取代(见 palette.ts)。docs/modules/ui-polish.md §1。
 *
 * 数值含义(全部归一到 0..1,OKLCH 的 L/C 与 alpha):
 *   L1..L8        8 级亮度锚(中性灰从这里取,chroma=0;强调色也以此为亮度基)。
 *   C1..C8        8 级色度锚(仅强调色用;中性恒为 0)。
 *   LWhite        「纯白/前景白」基准亮度(半透明白叠层用)。
 *   LForeground   前景文字极值亮度(memory/对比用)。
 *   LBackground   背景基准亮度(整窗底)。
 *   A*            各类半透明叠层的 alpha 锚点。
 * ========================================================================== */

/**
 * 一份明暗模式的 OKLCH 锚点(对应 PCL `ToneProfile` record 的字段与默认值)。
 * L/C 在 OKLCH 下取 0..1;hue 不在此存(由 ThemeAccent 提供),中性恒 chroma 0。
 */
export interface ToneProfile {
  L1: number;
  L2: number;
  L3: number;
  L4: number;
  L5: number;
  L6: number;
  L7: number;
  L8: number;
  /** 纯白/前景白基准亮度。 */
  LWhite: number;
  /** 前景文字极值亮度。 */
  LForeground: number;
  /** 背景基准亮度。 */
  LBackground: number;
  C1: number;
  C2: number;
  C3: number;
  C4: number;
  C5: number;
  C6: number;
  C7: number;
  C8: number;
  /** 半透明白叠层 alpha(约 73%)。 */
  ASemiWhite: number;
  /** 半透明白叠层 alpha(约 33%)。 */
  AHalfWhite: number;
  /** 强调极半透明 alpha(近不可见,用于 SemiTransparent)。 */
  ASemiTransparent: number;
  /** 半透明叠层 alpha(50%)。 */
  AHalfTransparent: number;
  /** 全透明 alpha(0)。 */
  ATransparent: number;
  /** 背景叠层 alpha。 */
  ABackground: number;
  /** ToolTip 叠层 alpha。 */
  AToolTip: number;
}

/**
 * PCL `ToneProfile` record 的默认值(= 浅色档)。逐字段对齐 ToneProfile.cs。
 * 浅色档亮度自暗(L1=0.35)向亮(L8=0.96)爬升,色度在中段(C2..C4)最盛。
 */
export const LIGHT_TONE: ToneProfile = {
  L1: 0.35,
  L2: 0.5,
  L3: 0.575,
  L4: 0.65,
  L5: 0.8,
  L6: 0.92,
  L7: 0.94,
  L8: 0.96,
  LWhite: 1,
  LForeground: 0,
  LBackground: 0.995,
  C1: 0.025,
  C2: 0.188,
  C3: 0.213,
  C4: 0.168,
  C5: 0.093,
  C6: 0.036,
  C7: 0.028,
  C8: 0.018,
  ASemiWhite: 0.733,
  AHalfWhite: 0.333,
  ASemiTransparent: 0.004,
  AHalfTransparent: 0.5,
  ATransparent: 0,
  ABackground: 0.824,
  AToolTip: 0.9,
};

/**
 * PCL `ToneProfileConfig.DefaultDark`:仅覆写亮度类锚点(L1..8 反转为亮→暗,
 * LBackground/LForeground/LWhite 调到深色档),色度与 alpha 锚点沿用浅色档默认。
 * 深色档 L1=0.96(最亮,作前景文字)向 L8=0.2(最暗,作底)递减——与浅色相反,
 * 生成器据此把中性映射回我们「--n-1=最暗底 → --n-8=最亮前景」的既有语义。
 */
export const DARK_TONE: ToneProfile = {
  ...LIGHT_TONE,
  L1: 0.96,
  L2: 0.75,
  L3: 0.6,
  L4: 0.65,
  L5: 0.45,
  L6: 0.25,
  L7: 0.225,
  L8: 0.2,
  LBackground: 0.3,
  LForeground: 1,
  LWhite: 0.275,
};

/** 明暗模式键。与 ipc/types.ts 的 ThemeMode 对齐('system' 由上层解析为其一)。 */
export type ToneMode = "dark" | "light";

/** 取某模式的锚点表。 */
export function toneFor(mode: ToneMode): ToneProfile {
  return mode === "dark" ? DARK_TONE : LIGHT_TONE;
}

/**
 * 强调色三元组(= PCL `GetCurrentThemeArgs()` 的 `(Hue, LightAdjust, ChromaAdjust)`)。
 * 取代现有扁平 `{hue,saturation,lightness}`:
 *   - hue           OKLCH 色相(0..360)。
 *   - lightAdjust   亮度旋钮(-1..1):正提亮、负压暗,经非对称 _AdjustLinear 不溢出。
 *   - chromaAdjust  色度旋钮(-1..1):正增艳、负去饱和。
 */
export interface ThemeAccent {
  hue: number;
  lightAdjust: number;
  chromaAdjust: number;
}

/**
 * PCL 内置主题的强调三元组(对应 ThemeService.GetCurrentThemeArgs() 的 switch)。
 * 作为预设的「感知化」锚点,供上层在需要时直接取用。
 */
export const ACCENT_PRESETS: Record<string, ThemeAccent> = {
  /** 天蓝(PCL SkyBlue)。 */
  skyBlue: { hue: 235, lightAdjust: 0.36, chromaAdjust: 0.2 },
  /** 猫猫蓝(PCL CatBlue,默认招牌)。 */
  catBlue: { hue: 255, lightAdjust: 0, chromaAdjust: -0.2 },
  /** 深蓝(PCL DeathBlue)。 */
  deathBlue: { hue: 268, lightAdjust: -0.05, chromaAdjust: -0.1 },
  /** HMCL 蓝(PCL HmclBlue)。 */
  hmclBlue: { hue: 275, lightAdjust: -0.03, chromaAdjust: -0.35 },
};
