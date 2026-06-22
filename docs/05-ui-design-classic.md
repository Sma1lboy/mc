# 05 · UI 设计 · 经典视图(SolidJS + CSS 落地)

> 当前产品方向不是复刻或命名为参考项目风格。本文把 PCL2 源码里的可调主题、紧凑顶栏与动效手感提炼为参考,再翻译成我们自己的 Tauri + SolidJS 经典视图设计系统。
> 提取自 `ref/PCL2/Plain Craft Launcher 2/Application.xaml`、`FormMain.xaml`、`Controls/My*`。

---

## 1. 参考设计语言提炼

看完源码,PCL 的辨识度来自这 7 条,缺一不可:

1. **标题栏即主导航**:顶部主题色横条,放一行 Tab(启动/下载/联机/设置/更多),白色图标+文字。主导航**不在左侧**——这是 PCL 和 Prism/官启最直观的区别。
2. **页内左右分栏**:每个页面 = 左侧二级列表/导航(`PanLeft`)+ 右侧内容(`PanMain`)。
3. **可调色相的主题色**:一套 HSL 三滑块(色相/饱和/明度)实时重算**整套色板**。默认蓝(`#1370f3`),用户可一键变绿/粉/紫。这是 PCL 的灵魂特性。
4. **数字色阶**:不是散落的颜色,而是 `Color1~8`(主题色明暗梯度)+ `Gray1~8`(灰阶)+ 语义红。所有控件引用色阶号,换主题色全局自动变。
5. **微圆角 + 卡片**:圆角只有 **3px**(清爽锐利,不是大圆角!),白卡片 + 柔和投影(`MyDropShadow`),浮在背景上。
6. **可自定义背景**:内容区底层一张背景图/纯色(`ImgBack` canvas),卡片半透明叠在上面。
7. **顺滑动画**:卡片滑入、Tab 切换、Hover 颜色过渡,弹性缓动,~200-400ms。左下角 Toast 滑入提示。

字体:`PCL English` + `Microsoft YaHei UI`,基准字号小(**12-13px**)。

---

## 2. 整体布局结构

```
┌──────────────────────────────────────────────────────┐
│ [Logo PCL]   启动  下载  联机  设置  更多      — × │ ← 标题栏(主题色渐变, 可拖拽)
├────────────┬─────────────────────────────────────────┤
│            │                                          │
│  PanLeft   │              PanMain                     │
│  (二级导航 │           (内容卡片区)                    │
│   /列表)   │        浮在 ImgBack 背景上                 │
│            │                                          │
│            │                                          │
│  [启动按钮]│                                          │ ← 启动页:左下角大启动按钮
├────────────┴─────────────────────────────────────────┤
│ Toast 从左下滑入                                        │
└──────────────────────────────────────────────────────┘
```

对应 CSS Grid:

```css
.app {
  display: grid;
  grid-template-rows: 48px 1fr;          /* 标题栏 + 内容 */
}
.page {
  display: grid;
  grid-template-columns: 280px 1fr;       /* 左栏 + 右内容 */
}
```

> Tauri 注意:`tauri.conf.json` 设 `"decorations": false` 用无边框窗口,自己画标题栏;标题栏区域加 `data-tauri-drag-region` 实现拖拽;关闭/最小化按钮调 Tauri window API。

---

## 3. 设计 Token(CSS 变量)

把 PCL 的 `ColorBrush1~8` / `Gray1~8` 映射成 CSS 变量。**所有组件只引用变量,不写死颜色**——这是主题色可调的前提。

```css
:root {
  /* ===== 主题色阶(由 H/S/L 生成,见 §4)。1=深 → 8=浅 ===== */
  --c-1: #0b5bcb;   /* 最深:按下态、强调文字 */
  --c-2: #1370f3;   /* 默认主题色(标题栏、主按钮) */
  --c-3: #4890f5;   /* Hover */
  --c-4: #6ba5f7;
  --c-5: #96c0f9;   /* 浅:选中底色 */
  --c-6: #d5e6fd;
  --c-7: #e0eafd;   /* 很浅:Hover 底 */
  --c-8: #eaf2fe;   /* 最浅:卡片选中/区块背景 */

  /* ===== 灰阶 ===== */
  --g-1: #343d4a;   /* 主文字 */
  --g-2: #404040;
  --g-3: #737373;   /* 次要文字 */
  --g-4: #8c8c8c;
  --g-5: #a6a6a6;   /* 占位/禁用 */
  --g-6: #cccccc;   /* 边框 */
  --g-7: #ebebeb;   /* 分隔线 */
  --g-8: #f5f5f5;   /* 区块背景 */

  /* ===== 语义 ===== */
  --red-light: #ff4c4c;
  --red-dark:  #ce2111;
  --bg-card:   rgba(255,255,255,0.92);  /* 半透明卡片 */
  --bg-window: #ffffff;

  /* ===== 形状/字体/动效 ===== */
  --radius:     3px;     /* PCL 招牌微圆角 */
  --radius-lg:  6px;     /* 大卡片 */
  --font:       "PCL English","Microsoft YaHei UI",system-ui,sans-serif;
  --fs-base:    13px;
  --fs-sm:      12px;
  --shadow:     0 2px 8px rgba(0,0,0,0.10);
  --shadow-hover: 0 4px 16px rgba(0,0,0,0.16);
  --ease:       cubic-bezier(0.4, 0, 0.2, 1);
  --dur:        280ms;
}
```

---

## 4. 主题色可调(灵魂特性)🌟

PCL 用 3 个滑块(`UiLauncherHue/Sat/Light`)实时生成整套色阶。我们在前端用一个函数从 HSL 基色推导 `--c-1~8` 注入到 `:root`:

```ts
// theme.ts —— 从基色 HSL 生成 8 级主题色阶
function applyThemeColor(hue: number, sat: number, light: number) {
  // 基色为 --c-2(默认经典蓝 ≈ h214 s90 l51)
  // 1~8 沿明度轴铺开:深→浅,饱和度略降以保持通透
  const scale = [
    { l: light - 14, s: sat },        // c-1 深
    { l: light,      s: sat },        // c-2 基色
    { l: light + 8,  s: sat - 4 },    // c-3 hover
    { l: light + 16, s: sat - 8 },
    { l: light + 26, s: sat - 14 },   // c-5 选中
    { l: light + 38, s: sat - 26 },
    { l: light + 44, s: sat - 34 },   // c-7
    { l: light + 48, s: sat - 40 },   // c-8 最浅
  ];
  const root = document.documentElement.style;
  scale.forEach((s, i) =>
    root.setProperty(`--c-${i+1}`, `hsl(${hue} ${clamp(s.s)}% ${clamp(s.l)}%)`)
  );
}
// 用户拖滑块 → applyThemeColor(h,s,l) → 全 UI 瞬间换色,无需重渲染
```

- 提供预设(经典蓝/绿/粉/紫/橙)+ 自定义滑块。
- 持久化到核心配置,启动时注入。
- 进阶:跟随壁纸取色、跟随系统强调色。

---

## 5. 签名组件清单(PCL → SolidJS)

| PCL 控件 | 我们的组件 | 关键样式 |
|----------|-----------|----------|
| `MyCard` | `<Card>` | 半透明白底 + `--shadow` + 3px 圆角 + 滑入动画 |
| `MyListItem` | `<ListItem>` | Hover 底色 `--c-7`、选中 `--c-8` + 左侧主题色条 |
| `MyButton`(主按钮) | `<Button variant="primary">` | 主题色 `--c-2`,Hover `--c-3`,按下 `--c-1` |
| `MyIconButton` | `<IconButton>` | 标题栏白色图标按钮,Hover 半透明白底 |
| `MyRadioButton`(顶部 Tab) | `<NavTab>` | 标题栏 Tab,选中下划线/底色 |
| `MyHint` | `<Toast>` | 左下角滑入,信息/成功/警告/错误配色 |
| `MyLoading` | `<Spinner>` | 主题色旋转动画 |
| `MySearchBox` | `<SearchBox>` | 圆角输入 + 图标 |
| `MySlider` | `<Slider>` | 主题色滑块(主题色调节本身就用它) |
| `MyComboBox` | `<Select>` | 下拉,选中项主题色 |
| `MyDropShadow` | CSS `box-shadow` | 不用组件,直接变量 |

启动页核心组件:**左下角大启动按钮** + 版本选择器(点击展开版本列表)。

```css
.launch-btn {
  /* PCL 招牌:左下角醒目大按钮 */
  height: 56px; min-width: 280px;
  background: var(--c-2);
  border-radius: var(--radius);
  color: #fff; font-size: 18px;
  box-shadow: var(--shadow);
  transition: background var(--dur) var(--ease), transform 120ms var(--ease);
}
.launch-btn:hover  { background: var(--c-3); }
.launch-btn:active { background: var(--c-1); transform: translateY(1px); }
```

---

## 6. 动画规范

PCL 的"顺滑感"来自克制但到位的动画:

| 场景 | 动画 | 时长/缓动 |
|------|------|-----------|
| 页面切换 | 内容横向滑入 + 淡入 | 280ms / `--ease` |
| 卡片出现 | 自下而上 8px + 淡入,列表项**错峰**(stagger 30ms) | 280ms |
| Hover | 背景色 + 阴影过渡 | 150ms |
| Toast | 左下滑入 → 停留 → 淡出 | 进 300ms / 出 200ms |
| 主题色切换 | CSS 变量变化,所有引用处自动过渡 | 200ms |
| 按钮按下 | `translateY(1px)` + 深色 | 120ms |

SolidJS 用 `solid-transition-group` 做进出场;列表 stagger 用 CSS `animation-delay: calc(var(--i) * 30ms)`。尊重 `prefers-reduced-motion`。

---

## 7. 前端工程结构

```
desktop/ui/
├── src/
│   ├── App.tsx
│   ├── theme/
│   │   ├── tokens.css         # §3 全部 CSS 变量
│   │   └── theme.ts           # §4 主题色生成 + 持久化
│   ├── layout/
│   │   ├── TitleBar.tsx       # 标题栏 + 主导航 Tab + 窗口控件
│   │   ├── Page.tsx           # 左右分栏骨架
│   │   └── Background.tsx     # 可自定义背景层
│   ├── components/            # §5 Card/Button/ListItem/Toast/...
│   ├── pages/
│   │   ├── Launch/            # 启动页(版本选择 + 大启动按钮)
│   │   ├── Download/          # 下载(版本/Mod/整合包)
│   │   ├── Link/             # 联机
│   │   └── Settings/
│   └── ipc/                   # 调 mc-core 的 tauri command 封装 + event 订阅
└── package.json               # SolidJS + Vite
```

技术点:
- **SolidJS**:细粒度响应式,无 VDOM diff,渲染快、包小 —— 和"性能优先"一致。
- **Vite** 构建,产物给 Tauri 打包。
- **状态**:SolidJS store;下载进度/日志这类流,订阅 §04 设计的 Tauri event 直接驱动信号。
- **图标**:PCL 用 SVG path(见 FormMain 里那串 path data),我们也用 SVG,可直接复用 PCL 的图标轮廓做风格统一。

---

## 8. 落地优先级(配合 §04 里程碑)

```
随 M5(Tauri 外壳)做:
  1. tokens.css + theme.ts(先把色阶和主题色调通)
  2. TitleBar(无边框窗口 + 拖拽 + 主导航 Tab + 窗口控件)
  3. Page 左右分栏骨架 + Background 层
  4. Card / Button / ListItem / Toast(四大基础件)
  5. 启动页:版本选择器 + 大启动按钮 + 进度/日志(接 M3 核心)
之后:
  6. 主题色滑块设置页(灵魂特性)
  7. 各业务页面(下载/Mod/联机/设置)
  8. 动画打磨 + 自定义背景 + 预设主题
```

> 关键:**第 1 步先把设计 token 和主题色生成跑通**,后面所有组件都基于变量,换肤零成本——这正是 PCL 能做到"一键全局换色"的工程基础。
