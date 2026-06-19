import { invoke } from "@tauri-apps/api/core";

/* ============================================================================
 * theme.ts —— PCL 式主题引擎
 *
 * 两个正交维度:
 *   1) 明暗模式 mode: 'dark' | 'light'  —— 只切换 --n-* 那组(由 tokens.css 按
 *      [data-theme] 选择器决定)。
 *   2) 强调色 accent: 由一组 HSL(hue/saturation/lightness)实时生成 --a-1..8
 *      整套色阶并注入 :root,所有引用 --a-* 的组件瞬间换色,无需重渲染。
 *
 * 这正是 PCL「一键全局换肤」的灵魂:组件只引用色阶号,换色只改变量。
 * ========================================================================== */

/** 后端 get_theme()/set_theme() 的配置形状,与 Tauri 命令约定一致。 */
export interface ThemeConfig {
  mode: "dark" | "light";
  hue: number; // 0..360
  saturation: number; // 0..100
  lightness: number; // 0..100(作为 --a-4 基色的明度锚点)
}

/** 默认主题:深色 + Modrinth 绿(h150 s60 l45)。invoke 失败时的兜底。 */
export const DEFAULT_THEME: ThemeConfig = {
  mode: "dark",
  hue: 150,
  saturation: 60,
  lightness: 45,
};

/** 把数值夹到合法百分比区间 [0,100]。 */
function clamp(v: number): number {
  return Math.max(0, Math.min(100, Math.round(v)));
}

/**
 * 从基色 HSL 生成 8 级强调色阶并注入 document.documentElement。
 *
 * 算法参考 docs/05 §4:以 lightness 为 --a-4 基色锚点,沿明度轴向两侧铺开
 * (1 深 → 8 浅),越浅的级数饱和度略降以保持「通透」不糊。
 *
 * 这样无论基色明暗,1~8 都能形成稳定的「深选中底 → 浅区块底」梯度,
 * 供「选中条/主按钮/hover/浅底」等语义直接取用。
 */
export function applyThemeColor(
  hue: number,
  saturation: number,
  lightness: number,
): void {
  // 每一级:相对基色明度的偏移 dl,以及相对基色饱和度的偏移 ds。
  // 基色 (--a-4) 落在第 4 级,故第 4 级偏移为 0。
  const scale: { dl: number; ds: number }[] = [
    { dl: -30, ds: 6 }, // a-1 最深:选中底/标记(更深更饱和一点更扎实)
    { dl: -18, ds: 4 }, // a-2
    { dl: -9, ds: 2 }, // a-3
    { dl: 0, ds: 0 }, // a-4 基色:主按钮/选中条
    { dl: 12, ds: -8 }, // a-5 hover
    { dl: 26, ds: -20 }, // a-6
    { dl: 40, ds: -34 }, // a-7
    { dl: 52, ds: -46 }, // a-8 最浅:浅底
  ];

  const root = document.documentElement.style;
  scale.forEach((s, i) => {
    const l = clamp(lightness + s.dl);
    const sat = clamp(saturation + s.ds);
    root.setProperty(`--a-${i + 1}`, `hsl(${hue} ${sat}% ${l}%)`);
  });
}

/** 切换明暗模式:写到 <html data-theme>,tokens.css 据此切 --n-* 那组。 */
export function setMode(mode: "dark" | "light"): void {
  document.documentElement.dataset.theme = mode;
}

/** 预设主题色(绿/蓝/粉/紫/橙)。lightness 作为基色明度锚点。 */
export const PRESETS: {
  name: string;
  hue: number;
  saturation: number;
  lightness: number;
}[] = [
  { name: "绿", hue: 150, saturation: 60, lightness: 45 }, // Modrinth 绿(默认)
  { name: "蓝", hue: 214, saturation: 88, lightness: 52 }, // PCL 蓝
  { name: "粉", hue: 330, saturation: 70, lightness: 58 },
  { name: "紫", hue: 265, saturation: 60, lightness: 58 },
  { name: "橙", hue: 28, saturation: 85, lightness: 54 },
];

/** 同时应用一份完整 ThemeConfig(模式 + 强调色)。 */
export function applyTheme(cfg: ThemeConfig): void {
  setMode(cfg.mode);
  applyThemeColor(cfg.hue, cfg.saturation, cfg.lightness);
}

/**
 * 启动时初始化主题:向后端取持久化配置并应用。
 * 后端不可用(开发期未起 Tauri / 命令未实现)时,回落默认主题,保证 UI 可用。
 * 返回实际生效的配置,供设置页初始化滑块。
 */
export async function initTheme(): Promise<ThemeConfig> {
  try {
    const cfg = await invoke<ThemeConfig>("get_theme");
    // 防御:后端字段缺失时逐项回落默认值,避免注入 NaN。
    const safe: ThemeConfig = {
      mode: cfg?.mode === "light" ? "light" : "dark",
      hue: Number.isFinite(cfg?.hue) ? cfg.hue : DEFAULT_THEME.hue,
      saturation: Number.isFinite(cfg?.saturation)
        ? cfg.saturation
        : DEFAULT_THEME.saturation,
      lightness: Number.isFinite(cfg?.lightness)
        ? cfg.lightness
        : DEFAULT_THEME.lightness,
    };
    applyTheme(safe);
    return safe;
  } catch {
    // get_theme 失败(如非 Tauri 环境):用默认主题兜底,不抛错阻塞渲染。
    applyTheme(DEFAULT_THEME);
    return DEFAULT_THEME;
  }
}

/**
 * 持久化主题到后端,并立即在前端生效(乐观更新)。
 * 写盘失败时仍保留前端已生效的视觉变更,但向上抛错以便调用方提示。
 */
export async function saveTheme(cfg: ThemeConfig): Promise<void> {
  // 先本地生效,换肤即时无延迟。
  applyTheme(cfg);
  await invoke("set_theme", { cfg });
}
