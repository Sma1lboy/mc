# 06 · UI 布局融合 · 工作台视图 + 经典主题引擎

> 用户同时喜欢现代 launcher 的**布局/信息架构**和参考启动器里的**主题色引擎/质感**。本文把两者融合成我们自己的工作台视图 + 经典视图方向。
> 本文的**布局**部分覆盖 [05](./05-ui-design-classic.md) 的 §2 布局;[05] 的**主题 token / 可调主题色 / 动画**继续有效(并在此扩成深色)。

---

## 1. 融合策略:各取所长

| 维度 | 采用谁 | 理由 |
|------|--------|------|
| **整体布局 / 信息架构** | **Modrinth** | 三区布局(图标栏 + 内容 + 右侧栏)信息密度高、现代、可扩展 |
| **主导航形态** | **Modrinth**(左侧图标栏) | 比 PCL 标题栏 Tab 更省横向空间、能固定常用实例、可扩展更多入口 |
| **首页 dashboard** | **Modrinth** | "继续游玩 + 发现整合包"的 home 比 PCL 直接进版本列表更友好 |
| **配色基调** | **Modrinth(深色)为默认** | 深色更现代、护眼;但保留浅色(见 §4) |
| **主题色引擎** | **PCL** 🌟 | 可调色相 HSL 生成整套色阶,一键全局换色 —— PCL 的灵魂保留 |
| **色阶 token 体系** | **PCL** | `accent-1~8` + `gray/neutral-1~8` 数字色阶,组件只引用变量 |
| **动画质感** | **PCL** | 卡片滑入、stagger、Hover 过渡、左下 Toast |
| **圆角** | 折中 | 卡片 `8px`(偏 Modrinth),小控件 `4-6px`,比 PCL 的 3px 圆润些 |

**一句话**:工作台负责信息架构,经典引擎负责可调主题和紧凑质感。

---

## 2. 三区布局

```
┌──┬───────────────────────────────────────────────┬──────────────┐
│  │ [logo] 名  ← →   Home          ● 无实例运行      │              │ ← 顶栏(48px)
│图├───────────────────────────────────────────────┤   右侧栏     │
│标│                                                │  ContextBar  │
│栏│            主内容区 MainContent                 │  (~340px)    │
│  │         (Home dashboard / 各页面)              │              │
│64│   ┌─────────────────────────────────────┐      │ ▸ 当前账号    │
│px│   │ Welcome back!                       │      │ ▸ 好友        │
│  │   │ Jump back in → [最近实例行]          │      │ ▸ 新闻/动态   │
│🏠│   │ Discover → [整合包大卡网格]          │      │              │
│🧭│   └─────────────────────────────────────┘      │ (右栏内容随   │
│📚│                                                │  页面变化)    │
│─ │                                                │              │
│⬛│ ← 固定的实例图标                                │              │
│⬛│                                                │              │
│➕│ ← 添加实例                                      │              │
│  │                                                │              │
│⚙ │                                                │              │
│👤│ ← 设置 / 账号(底部)                            │              │
└──┴───────────────────────────────────────────────┴──────────────┘
```

CSS Grid:

```css
.app {
  display: grid;
  grid-template-columns: 64px 1fr;
  grid-template-rows: 48px 1fr;
  grid-template-areas:
    "rail topbar"
    "rail body";
}
.rail   { grid-area: rail; }      /* 左图标栏 */
.topbar { grid-area: topbar; }
.body {
  grid-area: body;
  display: grid;
  grid-template-columns: 1fr 340px;   /* 主内容 + 右侧栏 */
}
/* 右侧栏可按页面隐藏:某些页面 grid-template-columns: 1fr; */
```

> Tauri:仍是无边框窗口。顶栏放 `data-tauri-drag-region` + 自绘的 macOS 红绿灯位(或交给系统)+ 后退/前进 + 页标题 + 右侧状态。

---

## 3. 三区组件清单

### 3.1 左图标栏(Rail)

| 区 | 内容 |
|----|------|
| 顶 | App Logo |
| 主导航 | 🏠 主页 · 🧭 发现(下载/浏览)· 📚 库(实例列表) |
| 分隔线 | |
| 固定实例 | 用户置顶的实例图标(直接点进/启动),可拖拽排序 |
| 添加 | ➕ 新建/导入实例 |
| 底部 | ⚙ 设置 · 👤 账号头像(点开账号切换) |

- 图标按钮:默认中性色,Hover 半透明 accent 底,**选中态左侧 accent 竖条 + accent 图标**(Modrinth 的绿条 → 我们的可调 accent)。
- 实例图标支持运行中状态点(绿点)。

### 3.2 顶栏(TopBar)

`Logo + 名 | ← → | 当前页标题/面包屑 | (弹性空白) | ● 运行状态`

- 后退/前进:页面历史导航(SolidJS Router)。
- 运行状态:无实例运行 / "Building 运行中"+ 快速操作。

### 3.3 主内容(MainContent)—— Home Dashboard

对标 Modrinth 首页:

| 区块 | 组件 |
|------|------|
| 欢迎语 | `Welcome back!` + 副标题 |
| 继续游玩 | `Jump back in` —— 最近实例**行卡片**:图标 + 名 + 元信息(单人/在线人数、上次游玩、loader)+ MOTD/状态 + **Play 按钮** + ⋮ 菜单 |
| 发现整合包 | `Discover a modpack →` —— **大图卡片网格**:封面图 + 图标 + 标题 + 描述 + 统计(下载数 ❤ 分类标签) |

### 3.4 右侧栏(ContextBar)—— 随页面变化

Home 页时显示:
| 区块 | 组件 |
|------|------|
| Playing as | 当前账号选择器(头像 + 名 + 下拉切换) |
| Friends | 好友列表 + 在线状态点(若做联机/社交) |
| News | 新闻/公告 feed |

> 右侧栏是**上下文相关**的:实例详情页右栏可放实例信息/截图;下载页右栏可放筛选器。不需要时整列隐藏。

---

## 4. 深色 token + accent(扩展 [05] §3)

[05] 给的是 PCL 浅色 token。这里扩成 **深色默认 + 可调 accent**,同时保留浅色。结构不变,仍是数字色阶,组件引用变量。

```css
/* ===== 深色(默认,对标 Modrinth)===== */
:root, [data-theme="dark"] {
  /* 中性/背景阶:1=最暗底 → 8=最亮前景 */
  --n-1: #0e0f11;   /* 窗口最底 */
  --n-2: #16181c;   /* rail / topbar */
  --n-3: #1b1e23;   /* 主内容底 */
  --n-4: #23272e;   /* 卡片 */
  --n-5: #2c313a;   /* 卡片 hover / 输入框 */
  --n-6: #3a414c;   /* 边框 */
  --n-7: #8b929e;   /* 次要文字 */
  --n-8: #e8eaed;   /* 主文字 */

  /* accent 色阶:由 H/S/L 生成,默认工作台绿 ≈ h150 */
  --a-1: #0f3d2a;   /* 最深:选中底/标记 */
  --a-2: #155f3a;   /* (生成) */
  --a-3: #1f7a4d;
  --a-4: #28a062;   /* 基色:主按钮/选中条 */
  --a-5: #3fbf78;   /* hover */
  --a-6: #6fd79b;
  --a-7: #a8e8c4;
  --a-8: #d9f6e6;   /* 最浅:浅底 */

  --bg-card:   var(--n-4);
  --text:      var(--n-8);
  --text-dim:  var(--n-7);
}

/* ===== 浅色(经典视图,可切换)===== */
[data-theme="light"] {
  --n-1:#ffffff; --n-2:#f5f5f5; --n-3:#fafafa; --n-4:#ffffff;
  --n-5:#f0f0f0; --n-6:#e0e0e0; --n-7:#737373; --n-8:#343d4a;
  /* accent 同样由引擎生成,默认经典蓝 h214 */
}

/* ===== 共用形状/动效 ===== */
:root {
  --radius-card: 8px;     /* 卡片(偏 Modrinth) */
  --radius:      6px;     /* 控件 */
  --radius-sm:   4px;
  --font: "Inter","Microsoft YaHei UI",system-ui,sans-serif;
  --fs-base: 14px;        /* 比 PCL 略大,Modrinth 偏舒展 */
  --shadow: 0 2px 12px rgba(0,0,0,0.35);   /* 深色阴影更重 */
  --ease: cubic-bezier(0.4,0,0.2,1);
  --dur: 240ms;
}
```

**主题色引擎不变**([05] §4 的 `applyThemeColor(h,s,l)`):
- 用户调 hue → 重算 `--a-1~8` 注入 `:root`。深/浅模式只换 `--n-*` 那组。
- 预设:工作台绿(默认)/ 经典蓝 / 粉 / 紫 / 橙。
- `data-theme` 切换深浅;accent hue 独立可调。**两个维度正交**。

---

## 5. 风格取舍备忘(冲突点的决定)

| 冲突 | PCL | Modrinth | **决定** |
|------|-----|----------|----------|
| 主导航位置 | 标题栏横 Tab | 左侧图标栏 | **左侧图标栏** |
| 默认配色 | 浅色 | 深色 | **深色默认,可切浅色** |
| 圆角 | 3px 锐利 | ~10px 圆润 | **8px 卡片 / 4-6px 控件** |
| 首屏 | 直接版本列表 | Home dashboard | **Home dashboard** |
| 字号 | 12-13px 紧凑 | 14px 舒展 | **14px** |
| 强调色 | 可调蓝 | 固定绿 | **可调 accent(默认绿),保留经典主题引擎** |

> 保留下来的经典引擎能力:**可调主题色引擎 + 数字色阶 token + 动画质感 + 左下 Toast**。
> 工作台视图负责:**三区布局 + 图标栏导航 + 深色 + Home dashboard + 更圆的卡片**。

---

## 6. 前端结构调整(覆盖 [05] §7 的 layout 部分)

```
desktop/ui/src/
├── theme/
│   ├── tokens.css         # §4 深/浅 + accent 变量
│   └── theme.ts           # 主题色引擎 + data-theme 切换
├── layout/
│   ├── AppShell.tsx       # 三区 Grid 骨架
│   ├── Rail.tsx           # 左图标栏(主导航 + 固定实例 + 设置/账号)
│   ├── TopBar.tsx         # 顶栏(导航箭头 + 标题 + 状态)+ Tauri 拖拽
│   └── ContextBar.tsx     # 右侧上下文栏(按页面切换内容)
├── pages/
│   ├── Home/              # dashboard:Jump back in + Discover
│   ├── Discover/          # 浏览下载(整合包/Mod)
│   ├── Library/           # 实例列表
│   ├── Instance/          # 单实例详情(右栏放实例信息)
│   └── Settings/          # 含主题色/深浅切换
├── components/            # Card / InstanceRow / ModpackCard / PlayButton / Toast / AccountSwitcher / FriendItem ...
└── ipc/                   # 调 mc-core
```

关键组件(Modrinth 同款):
- `<InstanceRow>` —— Jump back in 的横行卡(图标/名/元信息/Play/⋮)
- `<ModpackCard>` —— Discover 的大图卡(封面/图标/标题/描述/统计标签)
- `<PlayButton>` —— 主 accent 色,带运行中态
- `<AccountSwitcher>` —— 右栏账号下拉
- `<Rail>` 的 `<RailIcon>` —— 选中态 accent 竖条

---

## 7. 落地优先级(更新 [04] M5)

```
M5 拆成:
  5a. tokens.css(深色+accent) + theme.ts(引擎 + 深浅切换)
  5b. AppShell 三区 Grid + Rail + TopBar + 无边框窗口拖拽
  5c. 基础组件:Card / InstanceRow / ModpackCard / PlayButton / Toast
  5d. Home dashboard(接 M3 核心:真实最近实例 + 启动 + 进度/日志)
  5e. ContextBar(账号切换器)
之后:
  6. Discover / Library / Instance / Settings 页面
  7. 主题色设置(hue 滑块 + 深浅切换 + 预设)
  8. 动画打磨 + 自定义背景 + 好友/新闻(若做社交)
```

> 原则不变:**先把 token 引擎 + 三区骨架跑通**,再填页面。换肤(深浅 + accent)零成本是经典主题引擎给的底子。
