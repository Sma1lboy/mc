# 模块 · GUI 细节打磨(主题色 / 亚克力 / 瀑布流 / 图标)

> 动画之外的「其它细节」可抽象层。来自对 PCL-CE `PCL.Core/UI/` 的梳理:这些 PCL 用整套服务实现的东西,
> 在现代 Web 平台上大多塌缩成一两行——**只有「感知化主题色引擎」值得真正自建一层**,其余靠平台。
> 配套:[ui-animation.md](./ui-animation.md)(动画层,优先级更高)、[05-ui-design-pcl.md](../05-ui-design-pcl.md)(UI 设计意图)。

## 优先级总览

| 优先级 | 层 | 策略 | 现状差距 |
|--------|----|------|----------|
| **P1 自建** | 感知化主题色引擎(可调主题色 🌟) | 移植 PCL 的 OKLCH ToneProfile 生成器,**严格优于**现有朴素 HSL `theme.ts` | `theme.ts` 用裸 HSL + 手调偏移数组 |
| **P2 平台** | 亚克力 / 毛玻璃 | `backdrop-filter`,**不**移植任何模糊算法 | 只有对话框遮罩一处 `blur(2px)`,无复用类、无降级 |
| **P2 平台** | 系统主题模式 | `matchMedia('(prefers-color-scheme)')` | `mode` 只有 dark\|light,无 system |
| **P3 平台** | 瀑布流 / masonry | CSS `columns` 优先,必要时 ~20 行 JS | 画廊全是定高网格 |
| **P3 平台** | 统一 Icon 组件 | 内联 `<svg>` + `currentColor` | SVG 散落 ~10 个 TSX,无注册表、无统一着色 |

## 1. 感知化主题色引擎(P1,唯一值得自建)🌟

PCL-CE `ThemeService + ToneProfile + LabColor + CatColorResource` 是个**有真价值**的感知色引擎:存一份 light/dark 的 `ToneProfile`(OKLCH 的 L/C/alpha 锚点),用 OKLCH 派生 8 级灰 + 11 级强调色,每级强调按 per-theme 的 `(Hue, LightAdjust, ChromaAdjust)` 三元组用一个**非对称** `_AdjustLinear` 微调,gamut 映射后写进资源字典 `ColorObjectN/ColorBrushN`。这 1:1 映射到:一个 culori 驱动的生成器,从 OKLCH 基色发出 `--a-1..8` / `--n-1..8` CSS 自定义属性——**这正是 PCL 的「可调主题色」**,且严格优于我们现在的裸 HSL `theme.ts`。

映射:

| PCL | Web |
|-----|-----|
| `ToneProfile`(L1..8/C1..8/LWhite/LBackground/alpha,light & dark 各一份) | 每模式一个 TS 常量字面量:OKLCH 锚点表,喂生成器 |
| `LabColor.FromLch` / OKLAB / `OklchChromaReduction` gamut 映射 | culori `oklch()` → `clampChroma(c,'oklch','rgb')` → sRGB hex;或目标现代 WebView 时直接发 `oklch(L C H)` |
| `GetCurrentThemeArgs()` 的 `(Hue, LightAdjust, ChromaAdjust)` | `ThemeAccent = {hue, lightAdjust, chromaAdjust}`,取代现有扁平 `{hue,saturation,lightness}` |
| `_AdjustLinear(v, adj)` 非对称 lerp | 逐字移植:`adj>0 ? v+(1-v)*adj : v+v*adj`(单个有符号旋钮提亮/压暗不溢出) |
| `_CalculateGrays(tone)` 17 个中性(chroma 0) | 生成 `--n-1..8` + 命名中性(`--bg` 等),从 L 曲线**派生**而非硬编码 16 个 hex |
| `_CalculateColors(tone,args)` 11 个强调(含 Bg0/Bg1/SemiTransparent) | 生成 `--a-1..8` + 强调染色表面 + 半透明强调(我们当前缺这些令牌) |
| `CatColorResource.Apply()` 写资源字典 | `document.documentElement.style.setProperty('--a-'+i, v)` —— `theme.ts` 已经这么做 |
| `LightGrayCache/DarkGrayCache` + `InvalidateGrayCache` | SolidJS `createMemo`(按输入自动缓存,免手动失效) |

建议模块:`theme/tone.ts`(light/dark 的 OKLCH 锚点表)+ `theme/palette.ts`(给 ToneProfile + ThemeAccent 发 `--n-*`/`--a-*` + 强调染色表面)。逐字移植 `_AdjustLinear`。用 culori `oklch()`+`clampChroma(...,'rgb')` 输出 gamut 安全的 sRGB hex(匹配现有 hex 令牌)。整条用 SolidJS 响应式驱动:`mode`/`accent` 信号 → `createMemo` 产派生调色板 → 一个 `createEffect` `setProperty` 写出。

## 2. 亚克力 / 毛玻璃(P2,纯平台)

PCL 三个 WPF `ShaderEffect` 类(`AdaptiveBlur/EnhancedBlur/OptimizedBlur` + `BlurBorder`)重写了采样高斯模糊,**整坨努力只为补 WPF 没有便宜 GPU 模糊这件事**。Web 上全塌缩成:

```css
.surface-acrylic {
  background: var(--surface-translucent);
  backdrop-filter: blur(var(--blur-r)) saturate(1.2);
  -webkit-backdrop-filter: blur(var(--blur-r)) saturate(1.2);
  border-radius: var(--radius-card);
}
```

`backdrop-filter` GPU 合成、免费,圆角交给 `border-radius`。**不要移植采样数学。** 把现有 `PclAccountDialog.css` 那条孤零零的 `blur(2px)` 换成这个令牌类,复用到 PCL 浅色卡片/对话框。

**唯一值得留的 PCL 思想是 `SamplingRate` 的精神**(性能逃生口,非算法):`backdrop-filter` 是 WebView 里最贵的绘制——在弱 GPU / `prefers-reduced-transparency` 下,用 `[data-blur="off"]` 把 `backdrop-filter` 换成扁平半透明填充。

## 3. 系统主题模式(P2,纯平台)

`SystemThemeHelper.IsSystemInDarkMode`(读 Windows 注册表)→ `window.matchMedia('(prefers-color-scheme: dark)')` + change 监听。给 `ThemeConfig.mode` 加 `'system'`,媒体查询变化时重跑调色板生成器。现在 `mode` 只有 dark\|light。

## 4. 瀑布流 / masonry(P3,平台优先)

`WaterfallPanel` 是最短列 masonry(`Measure/Arrange` 把每个子项放进当前最矮的列)。Web:

- **只读画廊**:CSS `columns: N`(固定)/ `column-width: <max>`(自适应,`floor((W+gap)/(item+gap))` 即 `column-width` 本身)+ `break-inside: avoid`。
- **顺序敏感 / 可交互重排**:CSS columns 是按列从上往下填(阅读顺序与 PCL 的左→右最短填不同),此时移植那 ~20 行最短列 JS 算法;或等 CSS Grid masonry 落地。

Discover 现用 `grid auto-fill`,对**等高卡片够用**;只有卡片变高时才换 columns。**别引 masonry npm 依赖。**

## 5. 统一 Icon 组件(P3,平台)

PCL `SvgIcon`(path-data 模型 + `IconBrush` 着色 + `StrokeThickness`)→ 内联 `<svg>` SolidJS 组件,`fill/stroke: currentColor`,尺寸用 width/height。`IconBrush` == CSS `color` + currentColor;`Stretch` == width/height + viewBox `preserveAspectRatio`。

落地:一个 `<Icon name=… />` 组件 + 注册表(`import.meta.glob('./icons/*.svg', {as:'raw'})` 或每图 TSX),全用 currentColor,把现在散在 ~10 个 TSX 里的内联 SVG 收口。图标的颜色过渡就是普通 CSS `transition: color`,读 [ui-animation.md](./ui-animation.md) 的动机令牌(PCL `SvgIcon.AnimateIconBrushTo` 是 120ms CubicEaseOut,对应 `--mo-ease-out` + 120ms),**不需 JS 引擎**。

## 6. 兼容性提醒

- 在你实际发布的 Tauri WebView 上验证 `backdrop-filter` 与原生 `oklch()`:Windows WebView2、macOS WKWebView、Linux WebKitGTK。若某目标 `oklch()` 滞后,§1 的 culori→hex 策略直接绕开;`backdrop-filter` 则用 §2 的扁平填充兜底。

## 7. 排期

```
(1) 主题色/Tone 引擎 [自建,P1]     —— 严格升级 theme.ts,可调主题色的真正落地
(2) 亚克力类 + 性能兜底 [平台,P2]
(3) 系统主题模式 [平台,P2]
(4) Icon 组件收口 [平台,P3]
(5) masonry(CSS columns)[平台,P3]   —— JS 最短列算法可选
```

> 动画层([ui-animation.md](./ui-animation.md))仍是首要;本表是其后的「其它细节打磨」backlog。
