# 模块 · GUI 动画层(把 PCL 的动画系统抽象到 Web)

> 把 PCL 著名的动画系统抽象成**前端可复用的一层**。PCL-CE 已经把 PCL2 那坨 1061 行的 `ModAnimation.vb`
> 拆成了干净的引擎(`IClock`/`IAnimatable`/`IValueProcessor`/`IEasing` 四个缝),这套分层几乎**原样**能落到我们的
> SolidJS/Web 前端——只是 `IClock`→`requestAnimationFrame`、`IUIAccessProvider`→无(Web 单 UI 线程)。
>
> 配套:[ui-polish.md](./ui-polish.md)(主题色/亚克力/瀑布流/图标等其它可抽象的 GUI 细节)、
> [05-ui-design-pcl.md](../05-ui-design-pcl.md) §6(更高层的动画设计意图,本文是其工程实现层)。

## 0. 现状与目标

**现状(`desktop/src`)**:零 JS 动画引擎,零共享动机令牌(只有 `tokens.css` 里两个标量 `--dur:240ms` + 一条 `--ease`),零过渡原语。动画 100% 散装:**~37 条 `transition` 散落在 18 个 .css 文件**(各自重打 `0.15s/0.18s/0.2s/240ms` 和 cubic-bezier)+ 7 个手写 `@keyframes`。唯一的 JS 动画是 `Toast.tsx`——用 `.ui-toast--leaving` 类 + 硬编码 `setTimeout(…,220)` 假装退场,必须人工和 CSS 的 200ms 保持同步。所有 `<Show>/<Switch>/<Match>` 切换(页面、`rightView`、对话框、`layoutMode`)都是**瞬间硬切**,列表筛选无 FLIP。

**目标**:一个**薄**的动画层(全部加起来 ~250 行)+ 一套动机令牌,既复现 PCL 的「手感」,又不把 PCL 的多线程引擎照搬过来。

| 缺口 | 影响 |
|------|------|
| 无动机令牌(只有 1 条曲线 1 个时长) | 「PCL 手感」无法集中调,两套布局没法共享/覆盖 |
| 无缓动库(只有 1 条 Material cubic-bezier) | OutBack 回弹 / OutElastic 弹性等 PCL 标志曲线无从表达 |
| 无打断原语(keyed-cancel) | 快速 hover/press 来回会打架/跳变 |
| 无 Presence(退场后再卸载) | 每个 `<Show>` 切换都瞬切;Toast 靠 setTimeout 硬凑 |
| 无 FLIP / stagger | 列表筛选/重排会跳;首屏无逐项入场 |
| 无 JS 驱动值动画 | 数字读数、拖拽跟随、弹簧指示器都做不了 |
| `prefers-reduced-motion` 只覆盖 CSS | 将来任何 WAAPI/rAF 动画都会无视它 |

## 1. PCL 怎么做的(参考)

- **PCL2 `ModAnimation.vb`**(单体,1061 行):一条独立动画线程跑紧循环(非 `CompositionTarget.Rendering`),每帧算 `DeltaTime` → `RunInUiWait` 切回 UI 线程 → `AniTimer(dt*AniSpeed)`。工作单元是 `AniData`,~25 个 `Aa*` 流式 helper(`AaX/AaOpacity/AaScale/AaColor/AaCode/AaStack…`)按名字归到 `AniGroupEntry`,**同名 group 再启动会取消旧的**(=打断/重定向)。引擎是**增量式**:每帧把 `ease.GetDelta(now%,last%)*Value` 当增量加到属性上,所以动画可叠加、可中途重定向。「手感」集中在 ~40 个缓动类,最常用的是 `AniEaseOutFluent`(幂曲线 ease-out,几乎一切的默认)、`AniEaseOutBack`(回弹)、`AniEaseOutElastic`(弹性);默认时长 **400ms**。
- **PCL-CE `PCL.Core/UI/Animation/`**(分层重构):一个全局 `AnimationService` 持一个 `IClock`(~60Hz Tick)+ N 个 worker;每 Tick 调 `animation.ComputeNextFrame(target)` 产出一个延迟闭包 `()=>target.SetValue(...)`,经 `IUIAccessProvider` 切到 UI 线程执行。`IAnimation` 有状态(NotStarted/Running/Completed/Canceled)+ `CurrentFrame`;进度 `=CurrentFrame/TotalFrames` 过 `IEasing.Ease(p)` 再过类型化 `IValueProcessor`。三种动画:`FromToAnimation`(补间)/`ActionAnimation`(一次性回调步)/`RunAnimation`(XAML 触发);两种组合:`Parallel`/`Sequential`。命名动画冲突即取消(同 PCL2 的 group-cancel)。

> 关键洞察:PCL 之所以要自建引擎,是因为 WPF 没有便宜的逐属性动画;**Web 已经有 CSS transition + WAAPI**,所以别为平台白送的东西付费。PCL 的多线程 Channel/worker 全是为了把计算切出 WPF UI 线程——在 Web 单线程下是纯开销,**删掉**。

## 2. PCL → Web 映射(本层的核心)

| PCL 概念 | Web 等价 | 备注 |
|----------|----------|------|
| `IClock` / `WpfCompositionTargetRenderingClock`(每帧 UI 线程回调) | **一个共享 `requestAnimationFrame` 循环** | rAF 就是它的孪生:vsync 同步、UI 线程、自带高精度时间戳 |
| `IUIAccessProvider.Invoke`(切 UI 线程) | **无 / 直接调用** | Web 单 UI 线程,rAF 已在其上;保留一行 `applyOnUi(fn)=>fn()` 仅作未来 OffscreenCanvas 的占位 |
| `AnimationService`(全局注册表 + N worker + channel) | **一个单例 ticker:`Set<Tween>` + 一条 rAF 循环** | 删掉 Channel/ProcessorCount/worker;集合空了就停 rAF(空闲零 CPU) |
| `_namedAnimations` 同名冲突取消 | **`Map<key, handle>`,启动同 key 取消旧的** | 这是「跟手」的命脉;取消时**从当前值(理想含速度)起新动画**,否则快速来回会跳 |
| `AnimationStatus`(4 态) | `'idle'\|'running'\|'finished'\|'cancelled'`(对齐 WAAPI `playState`) | WAAPI 自带 `.finished` Promise + `.cancel()` |
| `FromToAnimation`(From/To/Duration/Easing/Delay) | DOM 属性 → CSS transition 或 `el.animate()`;非 DOM 值 → TS 补间调 `onUpdate(v)` | `ValueType.Relative` → `composite:'add'` / 偏移叠加 |
| `ComputeNextFrame`→延迟闭包 | `tick(now)`:`p=clamp((now-start-delay)/dur)`;`v=lerp(from,to,ease(p))`;`onUpdate(v)` **就地** | 同线程,无需延迟闭包间接层 |
| `IValueProcessor<T>`(Add/Sub/Scale) | 每类型 `lerp`:`lerpNumber/lerpColor/lerp 元组(transform)` | DOM 动画交给浏览器插值;只有 JS 驱动值才需 TS 插值器 |
| `Parallel/SequentialAnimationGroup` | `parallel(...)=Promise.all(h.finished)` / `sequence(...)=await` 链(用 `AbortSignal` 短路取消) | |
| `ActionAnimation`(时间轴里跑代码) | 序列里一个零时长步 `{run:()=>{sideEffect(); return resolved}}` | 「淡出→换内容→淡入」正是 `layoutMode` 切换所需 |
| `RunAnimationAction`(XAML 触发) | SolidJS `use:motion` 指令 / `<Motion>` / `createTween()` | Solid 无 XAML 触发,用指令 + Presence |
| `Fps`/`Scale`/`Duration→TotalFrames` 预算帧 | **基于时间的进度**(`elapsed/dur`),无全局 Fps | PCL 帧计数在掉帧时失真;rAF 时间戳保持正确。`Scale` → 全局 `motionScale` + 接 reduced-motion |

## 3. 引擎 vs 平台 切分(核心决策)

**不要照搬 `AnimationService`。** ~90% 的动画(hover/press/颜色/展开/对话框淡入/页面淡入)都能用 CSS transition(信号切类)或一次性 `el.animate()` 表达——让浏览器在合成线程插值。**只**为 CSS/WAAPI 做不到的 ~10% 建 TS rAF 引擎:

1. **可打断、从当前值重定向**的值补间(弹簧、拖拽跟随);
2. **FLIP** 列表重排;
3. **Presence**(退场后再卸载)的序列;
4. **overshoot/elastic/spring** 等 cubic-bezier 表达不了的缓动。

判据按**缓动可表达性**切,不按值类型切:多项式/正弦/指数/圆 族 → CSS `cubic-bezier()`/WAAPI(浏览器插值,可离主线程);overshoot/振荡/复合/弹簧 族 → rAF 采样 JS 缓动写 **CSS 自定义属性**(或信号)。现代浏览器还支持把 JS 缓动**预采样**成 WAAPI `linear()` easing——优先走它(仍离主线程)。

## 4. 动机令牌(先做,ROI 最高、零风险)

把 `tokens.css` 的 `{--dur,--ease}` 扩成一套**命名**令牌,并在 TS 里**镜像同名同值**,让 CSS 路径与 JS 路径共享一个真相源:

```css
/* theme/tokens.css */
--mo-dur-instant: 80ms;  --mo-dur-fast: 150ms;  --mo-dur-base: 200ms;
--mo-dur-page: 320ms;    --mo-dur-slow: 400ms;  --mo-dur-spring: 700ms;
--mo-ease-standard:  cubic-bezier(.4, 0, .2, 1);   /* 通用属性变化 */
--mo-ease-out:       cubic-bezier(.22, 1, .36, 1); /* = Fluent p3,PCL 默认手感基线 */
--mo-ease-emphasized:cubic-bezier(.16, 1, .3, 1);  /* 强 ease-out,入场/沉降 */
--mo-ease-accel:     cubic-bezier(.4, 0, 1, 1);    /* 退场 */
--mo-ease-back:      cubic-bezier(.34, 1.56, .64, 1); /* 回弹 */
--mo-stagger: 28ms;
```

然后扫一遍 18 个 .css,把手打的时长/曲线全换成 `var(--mo-*)`。这一步就把字面量散沙消掉、让「PCL 手感」一处可调(= PCL 把 `Time` 做成参数而非每处魔数)。

## 5. 缓动库(`motion/easings.ts`)

逐字从 PCL-CE 移植公式(**忠实即手感**,别用教科书 Penner 版替换 PCL 的非标准 Back/Elastic):

```ts
const clampEnds = (f: (t:number)=>number) => (t:number) => t<=0?0 : t>=1?1 : f(t); // 端点钳到精确 0/1
export const linear = (t:number)=>t;
export const easeOutFluent  = (t:number, p=3)=>1-(1-t)**p;                          // PCL 默认
export const easeOutBack    = (t:number, p=2)=>1-(1-t)**p*Math.cos(1.5*Math.PI*t);  // 回弹(cos 给过冲)
export const easeOutElastic = (t:number, p=2)=>{ const u=1-t;                        // 弹性(衰减正弦)
  return 1 - u**((p-1)*0.25) * Math.cos((p-3.5)*Math.PI*(1-u)**1.5); };
// Bounce:n1=7.5625,d1=2.75 四段;EasePower 枚举 Weak2/Middle3/Strong4/ExtraStrong5 → 指数
// CompositeEasing(多段加权同轴求和)/ CombinedEasing(split 处接续重标定)→ 纯 JS,无 CSS 等价
export const EASINGS: Record<string,(t:number)=>number> = { linear, /* …30 个命名,对齐 PCL EasingConverter */ };
```

- **哪些能进 CSS**:Quad/Cubic/Quart/Quint/Sine/Expo/Circ 的 In/Out/InOut → cubic-bezier 近似(Material 曲线);Back ≈ `cubic-bezier(.34,1.56,.64,1)`;**Elastic/Bounce/Composite/Spring 只能 JS**(或预采样成 `linear(...)`)。
- **颜色插值默认 sRGB 分量线性 lerp**(PCL `NColor` 就是 0..255 RGBA 的 Vector4 线性插值,**非感知**)——为对齐 PCL 观感,默认 sRGB;`{space:'oklch'}` 作可选感知升级(见 [ui-polish.md](./ui-polish.md))。

## 6. PCL「手感」预设表(从 `ModAnimation.vb` 提取)

逐条复现这些标志性动作(时长 + 曲线 + 动什么)。**热路径只动 `transform`/`opacity`**(PCL 随意动 Margin/Width 因 WPF 合成便宜;Web 会每帧重排)。`transform-origin: center`(= PCL `RenderTransformOrigin 0.5,0.5`)。

| 预设 | 时长 | 缓动 | 动什么 |
|------|------|------|--------|
| **page-enter** | 400(scale)+100(opacity) | OutBack(p2) / linear | `scale(.96→1)` 居中 + 淡入,轻过冲弹入 |
| **page-exit** | 110+80(延 30) | InFluent(Weak) / linear | `scale(1→.95)` + 淡出,快加速缩 |
| **list-stagger-enter** | 每项 100/300,延迟 `max(15-i,7)*2`ms | OutFluent → OutBack(Weak) | 每项 `translateX(-25→0)`(两段,末段过冲)+ 淡入 |
| **card/list-item hover** | 180 | OutFluent | 背景色交叉淡 + 极轻 `scale` |
| **list-item press** | ~162 | OutFluent | 整项 `scale(1→.98)` 挤压 + 背景变深 |
| **button press** | 80 按下 + 700 沉降(两段) | OutFluent(ExtraStrong p5)→(Middle p3) | `scale(1→.955)` 快挤 + 慢漂移 -0.01 |
| **icon-button hover** | 250 | OutBack(Weak)→OutFluent(Strong) | `scale→1.05` 过冲再回 1.0 |
| **checkbox/radio** | 150,勾在 *2 *0.7 延迟 | 边框 OutFluent;勾 OutBack | 边框缩入 → 勾/点 OutBack 过冲弹入(招牌弹) |
| **loading 镐挥动**(loop) | 350 蓄 + 900 挥 + 900 回 | InBack(蓄)→OutFluent(挥)→OutElastic(回) | `rotate` -20° 预备 → +50° 挥 → 弹性回 +25° 振荡 |
| **error 抖/弹** | 400(延 300) | OutBack | 红叉 `scale(0→1)` 过冲 + 红色淡入 |
| **scroll 惯性** | 300 | OutFluent(6)/带初速变体 | 滚动偏移减速(顺滑滚轮) |
| **hint/toast 退场** | 200 淡 + 150 收(延 100) | InFluent/OutFluent | 微缩 + 淡出 → 高度收起移除 |

对应到**我们的组件**(`our-frontend` agent 映射):`hoverLift`(Card/InstanceRow/ModpackCard)、`press`(Button/PlayButton/launchbtn `:active`,纯 CSS)、`pageEnter`(AppShell/PclShell `Switch/Match`,320ms 淡+移)、`viewCrossfade`(PclLaunch `rightView` news/versions/log)、`expandCollapse`(`grid-template-rows 0fr↔1fr` + 雪佛龙转 180°)、`dialog`(PclAccountDialog 遮罩淡 + 卡 `scale .96→1`)、`toast`(替掉 setTimeout)、`listStagger`/`listReflow(FLIP)`(版本列表、Discover 网格)、`layoutSwap`(App.tsx modrinth↔pcl 整壳交叉淡)、`selectIndicator`(单条滑动强调条)。`spin`/`skeletonPulse` 保持纯 CSS `@keyframes`。

## 7. TS API / 模块布局

```
desktop/src/motion/
  tokens.ts      DUR{instant,fast,base,page,slow,spring} + EASE{standard,out,emphasized,accel,back} —— 与 tokens.css 同值
  easings.ts     纯函数缓动(逐字移植 PCL)+ EASINGS 命名表 + Composite/Combined 组合子
  interpolators.ts  lerpNumber / lerpColor(sRGB 默认,oklch 可选)/ lerp 元组(transform/thickness)
  targets.ts     AnimTarget<T>{get,set}:DomStyleTarget(el,prop) / CssVarTarget(el,'--x') / SignalTarget(get,set) / nullTarget
  engine.ts      一条 rAF ticker over Set<Tween>;animate(opts)->Handle;keyed-cancel;空集停 rAF
  solid.ts       createTween / use:motion 指令 / <Presence> / <Motion> / stagger(items,step) / flip(container)
  reduced.ts     matchMedia('(prefers-reduced-motion: reduce)') 读一次 + 监听;reduced() 访问器;motionScale 信号
```

```ts
// engine.ts — 形状
interface AnimationHandle { status: 'running'|'finished'|'cancelled'; finished: Promise<void>; cancel(): void }
function animate(opts: {
  key?: string;                 // 同 key 启动会取消旧的(PCL 命名冲突取消)
  from?: number; to: number;    // from 省略 = 启动时读当前值(可打断、不跳)
  duration: number; delay?: number;
  ease: (t:number)=>number;
  onUpdate: (v:number)=>void;   // 写信号 / style / 自定义属性(= IAnimatable)
}): AnimationHandle;
```

```tsx
// solid.ts — SolidJS 面向用法
const [w, animateTo] = createTween(240);        // 动画化数字进 JSX
animateTo(360, { duration: DUR.base, ease: easeOutBack });

<div use:motion={{ preset: 'pageEnter' }}>…</div>   // 挂载即入场,卸载自动取消

<Presence>                                       // 退场后再卸载(修掉 Toast 的 setTimeout)
  <Show when={open()}><Dialog/></Show>
</Presence>

<For each={items()}>{(it, i) =>                  // 逐项错峰(= AaStack)
  <div style={{ '--i': i() }} use:motion={{ preset: 'listStagger' }}>…</div>}
</For>
```

## 8. SolidJS 集成要点

- **模块级单例,无 Context**(对齐 `store.ts` 约定):ticker 是模块单例(像 Toast store);`createTween`/`<Presence>`/`use:motion` 直接 import。**不要**引 `MotionProvider`。
- **先做 `<Presence>`,再做引擎**:最高价值的人体工学是「退场后卸载」——今天每个 `<Show>/<Switch>` 瞬切、Toast 用 `setTimeout(220)` 硬凑。`<Presence>` 用 WAAPI `.finished` 拦截卸载、跑退场、再卸,一举修好 Toast/对话框/页面切换/rightView/布局切换。这条只需 WAAPI,不需自研引擎。
- **JS 引擎自己honor reduced-motion**:`tokens.css` 的 `!important` 块只盖 CSS;rAF/WAAPI 会无视它。`reduced.ts` 读一次 + 监听,所有层 `dur *= reduced()?0:1`(reduced 时路由 `nullTarget` 并直接落终值)。
- **`.no-motion` 闸**(= PCL `AniControlEnabled`):初次挂载 / `layoutMode` 切换 / 状态恢复时加 `.no-motion` 抑制入场动画,下一帧移除。
- **不引重依赖**:`solid-transition-group`(官方、极小)可作 `<Transition>` 的参照/兜底,但我们的需求(Presence + FLIP + 一个数字补间)~250 行能自持,契合「from-scratch 薄层」与小包体取向。
- **两套布局同引擎、不同令牌值**:`pcl` 用上面忠实 PCL 的时长/曲线;`modrinth` 可更利落扁平。令牌做成 per-theme 覆盖,不分叉代码。

## 9. 路线图

1. **令牌先行**(零风险、最高 ROI):扩 `tokens.css` 成 `--mo-*` 时长尺 + 4 条命名缓动;扫 18 个 .css 换成 `var(--mo-*)`。
2. **镜像进 TS**(`motion/tokens.ts` + `reduced.ts`):同值,集中读 reduced-motion。
3. **`<Presence>`/`<Motion>` 先于引擎**:WAAPI 退场,修 Toast/对话框/页面/布局切换。
4. **`createTween` + rAF 引擎**:仅当出现弹簧/拖拽跟随/弹性指示器等 JS 驱动值时;移植 `FromToAnimationBase` 进度数学 + 端点钳 + keyed 打断(从当前值起)。
5. **缓动库 + FLIP + stagger**:逐字移植 PCL 曲线;FLIP 修列表重排;stagger 做首屏入场。

> 实现顺序的精神:**令牌 → Presence → 引擎**。先把散沙收成令牌、把瞬切换成有退场的过渡,再为少数真需要 JS 的场景上引擎。PCL 的「手感」在曲线与时长里(§4/§5/§6),不在线程管线里。
