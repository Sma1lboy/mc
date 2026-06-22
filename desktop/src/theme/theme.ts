import { invoke } from "@tauri-apps/api/core";
import { createSignal, createMemo, createRoot, createEffect } from "solid-js";
import { generatePalette, type Palette } from "./palette";
import { toneFor, type ThemeAccent, type ToneMode } from "./tone";

/* ============================================================================
 * theme.ts —— 主题引擎(感知化 / OKLCH 版)
 *
 * 两个正交维度:
 *   1) 明暗模式 mode: 'dark' | 'light' | 'system'
 *      —— 决定取哪份 ToneProfile(tone.ts);'system' 跟随 prefers-color-scheme。
 *   2) 强调色 accent: 由一组旋钮(hue/saturation/lightness,向后兼容的扁平形)
 *      派生出感知化的 ThemeAccent{hue,lightAdjust,chromaAdjust},喂生成器
 *      (palette.ts)发出整套 --n-1..8 + --a-1..8 + 强调表面令牌并注入 :root。
 *
 * 核心约束:组件只引用色阶号,换色只改变量;且此版用 OKLCH
 * 感知锚点 + 非对称 _AdjustLinear,严格优于旧朴素 HSL 偏移数组。docs/modules/ui-polish.md §1。
 *
 * 响应式管线(SolidJS,模块级单例,无 Context——对齐 store.ts 约定):
 *   themeConfig 信号 + systemDark 信号 → createMemo 派生 Palette → createEffect
 *   把每个 CSS 变量 setProperty 到 document.documentElement。
 * ========================================================================== */

/** 后端 get_theme()/set_theme() 的配置形状,与 Tauri 命令约定一致(向后兼容)。 */
export interface ThemeConfig {
  mode: "dark" | "light" | "system";
  hue: number; // 0..360
  saturation: number; // 0..100(映射到 OKLCH 色度旋钮)
  lightness: number; // 0..100(映射到 OKLCH 亮度旋钮)
}

/** 默认主题:深色 + 工作台绿(h150 s60 l45)。invoke 失败时的兜底。 */
export const DEFAULT_THEME: ThemeConfig = {
  mode: "dark",
  hue: 150,
  saturation: 60,
  lightness: 45,
};

/**
 * 各布局「对味」的默认主题。布局与主题是耦合的:工作台配深色+绿,
 * 经典视图配浅色+蓝。switchLayout / 启动注入 / 设置页「重置」都走这同一份,避免
 * 三处各写一份魔法数导致跑偏(例如经典布局却套深色,出现顶栏浅、正文深的诡异组合)。
 */
export const WORKSPACE_THEME: ThemeConfig = DEFAULT_THEME;
export const CLASSIC_THEME: ThemeConfig = {
  mode: "light",
  hue: 255,
  saturation: 40,
  lightness: 45,
};

const LEGACY_CLASSIC_ACCENTS = [
  { hue: 214, saturation: 88, lightness: 52 },
  { hue: 214, saturation: 90, lightness: 51 },
];

/** 取某布局相称的默认主题。 */
export function themeForLayout(layout: "workspace" | "classic"): ThemeConfig {
  return layout === "classic" ? CLASSIC_THEME : WORKSPACE_THEME;
}

/**
 * 当前主题是否仍等于某布局的默认(即用户从未自定义过配色)。
 * 切换布局时据此判断:未自定义才套用新布局默认皮肤,自定义过则原样保留。
 */
export function isThemeUntouchedFor(layout: "workspace" | "classic"): boolean {
  return sameTheme(themeConfig(), themeForLayout(layout));
}

function sameTheme(a: ThemeConfig, b: ThemeConfig): boolean {
  return (
    a.mode === b.mode &&
    Math.abs(a.hue - b.hue) < 0.5 &&
    Math.abs(a.saturation - b.saturation) < 0.5 &&
    Math.abs(a.lightness - b.lightness) < 0.5
  );
}

function sameAccent(
  a: Pick<ThemeConfig, "hue" | "saturation" | "lightness">,
  b: Pick<ThemeConfig, "hue" | "saturation" | "lightness">,
): boolean {
  return (
    Math.abs(a.hue - b.hue) < 0.5 &&
    Math.abs(a.saturation - b.saturation) < 0.5 &&
    Math.abs(a.lightness - b.lightness) < 0.5
  );
}

function numberOrDefault(value: unknown, fallback: number): number {
  const num = Number(value);
  return Number.isFinite(num) ? num : fallback;
}

function sanitizeThemeConfig(cfg: Partial<ThemeConfig> | null | undefined): ThemeConfig {
  return {
    mode: cfg?.mode === "light" || cfg?.mode === "system" ? cfg.mode : "dark",
    hue: clampRange(numberOrDefault(cfg?.hue, DEFAULT_THEME.hue), 0, 360),
    saturation: clampRange(numberOrDefault(cfg?.saturation, DEFAULT_THEME.saturation), 0, 100),
    lightness: clampRange(numberOrDefault(cfg?.lightness, DEFAULT_THEME.lightness), 20, 70),
  };
}

export function normalizeThemeConfig(
  cfg: Partial<ThemeConfig> | null | undefined,
  layout?: "workspace" | "classic",
): { config: ThemeConfig; changed: boolean } {
  const safe = sanitizeThemeConfig(cfg);
  const rawChanged =
    !cfg ||
    cfg.mode !== safe.mode ||
    cfg.hue !== safe.hue ||
    cfg.saturation !== safe.saturation ||
    cfg.lightness !== safe.lightness;

  if (LEGACY_CLASSIC_ACCENTS.some((accent) => sameAccent(safe, accent))) {
    return { config: CLASSIC_THEME, changed: true };
  }

  if (layout === "classic" && sameTheme(safe, DEFAULT_THEME)) {
    return { config: CLASSIC_THEME, changed: true };
  }

  return { config: safe, changed: rawChanged };
}

/* ----------------------------------------------------------------------------
 * 系统暗色偏好(P2:'system' 模式跟随之)。读一次 matchMedia + 监听变化,
 * 变化时驱动信号重算调色板(下方响应式管线自动重注入)。
 * -------------------------------------------------------------------------- */

const DARK_QUERY = "(prefers-color-scheme: dark)";

const colorSchemeMql: MediaQueryList | null =
  typeof window !== "undefined" && typeof window.matchMedia === "function"
    ? window.matchMedia(DARK_QUERY)
    : null;

const [systemDark, setSystemDark] = createSignal<boolean>(colorSchemeMql?.matches ?? true);

if (colorSchemeMql) {
  const onChange = (e: MediaQueryListEvent): void => {
    setSystemDark(e.matches);
  };
  if (typeof colorSchemeMql.addEventListener === "function") {
    colorSchemeMql.addEventListener("change", onChange);
  } else if (typeof (colorSchemeMql as MediaQueryList).addListener === "function") {
    // 已废弃 API,仅为老内核兜底。
    (colorSchemeMql as MediaQueryList).addListener(onChange);
  }
}

/** 把配置 mode + 系统偏好解析为实际生效的明暗档('system' → 跟随系统)。 */
function resolveMode(mode: ThemeConfig["mode"], sysDark: boolean): ToneMode {
  if (mode === "system") return sysDark ? "dark" : "light";
  return mode;
}

/* ----------------------------------------------------------------------------
 * 旋钮映射:扁平 {saturation,lightness}(向后兼容的设置滑块)→ 感知化 ThemeAccent。
 * hue 直接透传;saturation/lightness 居中线性映射到 [-1,1] 的 _AdjustLinear 旋钮,
 * 让既有滑块语义(更饱和/更亮)对应「增艳/提亮」,且端点不溢出。
 * -------------------------------------------------------------------------- */

/** 把扁平 HSL 旋钮换算成感知化的 ThemeAccent(hue/lightAdjust/chromaAdjust)。 */
export function accentFromHsl(
  hue: number,
  saturation: number,
  lightness: number,
): ThemeAccent {
  // saturation 0..100:50 为中性(锚点原色度),>50 增艳、<50 去饱和。
  const chromaAdjust = clampRange((saturation - 50) / 50, -1, 1);
  // lightness 20..70(滑块区间):45 为中性,越高越提亮、越低越压暗。
  const lightAdjust = clampRange((lightness - 45) / 25, -1, 1);
  return { hue: ((hue % 360) + 360) % 360, lightAdjust, chromaAdjust };
}

/** 把数值夹到 [lo,hi]。 */
function clampRange(v: number, lo: number, hi: number): number {
  if (!Number.isFinite(v)) return lo;
  return Math.max(lo, Math.min(hi, v));
}

/* ----------------------------------------------------------------------------
 * 响应式管线(模块级 createRoot,长生命周期,不随组件卸载销毁)。
 * -------------------------------------------------------------------------- */

const [themeConfig, setThemeConfigInternal] = createSignal<ThemeConfig>(DEFAULT_THEME);

/** 当前生效的 ThemeConfig(只读 accessor),供设置页/调试读取。 */
export const currentTheme = themeConfig;

// 在一个独立 root 里装派生 + 注入 effect(避免被组件作用域回收)。
createRoot(() => {
  // 派生:配置 + 系统偏好 → 解析档 → ToneProfile + ThemeAccent → Palette。
  const palette = createMemo<{ mode: ToneMode; vars: Palette }>(() => {
    const cfg = themeConfig();
    const mode = resolveMode(cfg.mode, systemDark());
    const tone = toneFor(mode);
    const accent = accentFromHsl(cfg.hue, cfg.saturation, cfg.lightness);
    return { mode, vars: generatePalette(tone, accent) };
  });

  // 注入:写 data-theme(供 tokens.css 的少量 [data-theme] 选择器兜底)+ 全部 CSS 变量。
  createEffect(() => {
    if (typeof document === "undefined") return;
    const { mode, vars } = palette();
    const root = document.documentElement;
    root.dataset.theme = mode;
    for (const [name, value] of Object.entries(vars)) {
      root.style.setProperty(name, value);
    }
  });
});

/* ----------------------------------------------------------------------------
 * 公共命令式 API(向后兼容:Settings/store/App 仍按旧签名调用,内部走响应式管线)。
 * -------------------------------------------------------------------------- */

/**
 * 从基色旋钮生成并注入强调色阶(向后兼容签名)。现在它只更新 themeConfig 的强调部分,
 * 真正的注入由上面的响应式 effect 完成(感知化 OKLCH 生成 + gamut 安全)。
 */
export function applyThemeColor(
  hue: number,
  saturation: number,
  lightness: number,
): void {
  setThemeConfigInternal((prev) => ({ ...prev, hue, saturation, lightness }));
}

/** 切换明暗模式(含 'system')。更新 themeConfig.mode,响应式管线据此重算。 */
export function setMode(mode: "dark" | "light" | "system"): void {
  setThemeConfigInternal((prev) => ({ ...prev, mode }));
}

/** 同时应用一份完整 ThemeConfig(模式 + 强调色)。 */
export function applyTheme(cfg: ThemeConfig): void {
  setThemeConfigInternal(cfg);
}

/** 预设主题色(绿/蓝/粉/紫/橙)。lightness 作为基色明度锚点(滑块语义保持)。 */
export const PRESETS: {
  name: string;
  hue: number;
  saturation: number;
  lightness: number;
}[] = [
  { name: "绿", hue: 150, saturation: 60, lightness: 45 }, // 工作台绿(默认)
  { name: "蓝", hue: 255, saturation: 40, lightness: 45 }, // 经典蓝
  { name: "粉", hue: 330, saturation: 70, lightness: 58 },
  { name: "紫", hue: 265, saturation: 60, lightness: 58 },
  { name: "橙", hue: 28, saturation: 85, lightness: 54 },
];

/**
 * 启动时初始化主题:向后端取持久化配置并应用。
 * 后端不可用(开发期未起 Tauri / 命令未实现)时,回落默认主题,保证 UI 可用。
 * 返回实际生效的配置,供设置页初始化滑块。
 */
export async function initTheme(layout?: "workspace" | "classic"): Promise<ThemeConfig> {
  try {
    const cfg = await invoke<ThemeConfig>("get_theme");
    const { config, changed } = normalizeThemeConfig(cfg, layout);
    applyTheme(config);
    if (changed) void invoke("set_theme", { cfg: config }).catch(() => {});
    return config;
  } catch {
    // get_theme 失败(如非 Tauri 环境):用默认主题兜底,不抛错阻塞渲染。
    const fallback = layout ? themeForLayout(layout) : DEFAULT_THEME;
    applyTheme(fallback);
    return fallback;
  }
}

/**
 * 持久化主题到后端,并立即在前端生效(乐观更新)。
 * 写盘失败时仍保留前端已生效的视觉变更,但向上抛错以便调用方提示。
 */
export async function saveTheme(cfg: ThemeConfig): Promise<void> {
  const { config } = normalizeThemeConfig(cfg);
  // 先本地生效,换肤即时无延迟。
  applyTheme(config);
  await invoke("set_theme", { cfg: config });
}
