# SolidJS → React 19 迁移约定

本仓库正在把 `desktop/` 前端从 **SolidJS** 迁到 **TypeScript + React 19**。本文件是
并行迁移 agent 的**唯一权威约定**。开工前通读;每个映射都配了本仓库的真实例子。

> 现状:阶段①(基座)已落地并单独 typecheck 通过。整棵树在迁移完成前**不会**整体
> 编译 —— 这是预期的:pages/components/layout/agent 仍是 Solid。只需保证你新写的
> React 文件本身干净。

---

## 0. 基座已就绪(可直接依赖)

| 能力 | 位置 | 用法 |
| --- | --- | --- |
| 全局状态 | `src/store.ts`(zustand) | 组件 `useAppStore((s) => s.x)`;非组件 `x()` getter |
| i18n | `src/i18n.ts`(zustand) | `t("ns.key", params?)` + 组件顶部 `useLang()` |
| 主题 | `src/theme/theme.ts`(**仍是 Solid,后续阶段移植**) | `initTheme()` 公开签名不变,runtime 仍工作 |
| 动画 | `src/motion/react.tsx` | `useEntrance(preset)`、`<Presence>`;新动画用 framer `motion`/`AnimatePresence` |
| 异步取数 | `src/util/useAsync.ts` | `createResource` 的替身 |
| 入口 | `src/main.tsx` + `src/App.tsx` | 已接通;阶段③把 App 的占位换成 `<AppShell/>` |

工具链:`vite-plugin-solid` → `@vitejs/plugin-react`(**v4 线**:v6 要求 vite 8,本项目
vite 5)。`tsconfig` `jsx: "react-jsx"`(自动运行时,**不需要** `import React`)。Solid 依赖
暂留(最后阶段删)。

新增依赖:`react`/`react-dom` 19、`zustand`、`@ark-ui/react`、`lucide-react`、
`@tanstack/react-virtual`、`motion`(framer 后继)、`streamdown`、`clsx`。
> ⚠️ **streamdown** 拉入 `mermaid`(重,~数百 KB)。仅在真正渲染流式 markdown 的
> 聊天页(`agent/ChatPage`)按需 import,别在通用组件里引它。它支持 React 18/19。

---

## 1. 核心映射(Solid → React)

### createSignal → useState（组件内）/ zustand（全局）
```tsx
// Solid
const [open, setOpen] = createSignal(false);
open();  setOpen(true);  setOpen(v => !v);
// React（组件局部状态）
const [open, setOpen] = useState(false);
open;    setOpen(true);  setOpen(v => !v);   // 注意:读值不再是 open()
```
组件间共享的状态**不要**用 useState 各自持有 —— 放进 `store.ts`(见 §2)。

### createMemo → useMemo
```tsx
// Solid: const total = createMemo(() => a() + b());  total()
const total = useMemo(() => a + b, [a, b]);   // 必须显式列 deps
```

### createEffect → useEffect（⚠️ deps 手动枚举)
Solid effect **自动追踪**依赖;React 必须手列 deps 数组。逐一列出 effect 体里读到的每个
响应值,否则会读到过期闭包或漏触发。
```tsx
// Solid: createEffect(() => { doThing(page()); });
useEffect(() => { doThing(page); }, [page]);
```
`onMount` → `useEffect(() => { … }, [])`;`onCleanup` → effect 的 return 清理函数。

### createResource → useAsync（`src/util/useAsync.ts`)
```tsx
// Solid
const [data] = createResource(() => root(), (r) => api.listX(r));
data();          // 值(未就绪 undefined)
data.loading;    // 是否在拉
// React
const { data, loading, error, refetch } = useAsync(() => api.listX(root), [root]);
```
resource 的 **source** 变化重拉 → useAsync 的 **deps** 变化重拉。全仓库约 20 处组件本地
`createResource` 都走这个替身(它们的 `.loading` 都是本地的,与 store 的 `instances` 无关)。

### JSX 控制流
```tsx
// <Show when={x} fallback={<A/>}><B/></Show>
{x ? <B/> : <A/>}
// <Show when={x}><B/></Show>
{x && <B/>}
// <For each={list()}>{(item) => <Row item={item}/>}</For>
{list.map((item) => <Row key={item.id} item={item} />)}   // key 必填,用稳定业务 id
// <Index each={list()}>{(item, i) => <Row v={item()} i={i}/>}</Index>
{list.map((v, i) => <Row key={i} v={v} i={i} />)}          // 按下标,item 不是函数
// <Switch><Match when={a}>…</Match><Match when={b}>…</Match></Switch>
{a ? … : b ? … : null}
// <Dynamic component={Comp} .../>  →  首字母大写的组件变量
const Comp = route.component; return <Comp {...props} />;
```
`<For>` 的 key:优先业务稳定 id(`item.id`)。列表纯静态/永不重排才可用下标。

### 事件名 —— ⚠️ onInput/onChange 语义反转
- Solid `onInput` = 原生 input 事件(每次击键)→ **React `onChange`**(React 已把 onChange
  归一为 input 语义,每次击键触发)。
- Solid `onChange` = 原生 change 事件(失焦/提交)→ **React `onBlur`**(或按需 onChange,
  但语义不同)。

全仓库表单几乎都用 Solid `onInput`(约 10 个文件),一律换成 React `onChange`:
```tsx
// Solid: <input value={q()} onInput={(e) => setQ(e.currentTarget.value)} />
<input value={q} onChange={(e) => setQ(e.currentTarget.value)} />
```

### classList → clsx（已装)
```tsx
// Solid: <div classList={{ hidden: !open(), active: sel() === id }} class="base" />
import clsx from "clsx";
<div className={clsx("base", { hidden: !open, active: sel === id })} />
```
`class` → `className`。字符串拼接的地方(如 Toast 里的模板串)保持模板串亦可,统一优先 clsx。

### refs
```tsx
// Solid: let el; <div ref={el} />;  onMount(() => el.focus());
const el = useRef<HTMLDivElement>(null);
<div ref={el} />;  useEffect(() => el.current?.focus(), []);
```
需要「挂载即跑」逻辑而不想 useRef+useEffect 时,可用 ref 回调(见 `useEntrance` 的实现)。

### children / props
```tsx
// Solid: props.children;  splitProps(props, ["class"])
function X({ children, className, ...rest }: XProps) { … }   // 解构即可
```
Solid 的 `children(() => props.children)` resolver 不需要;React children 是普通节点。

### Portal → createPortal
5 个组件用 `<Portal>`(Dialog/Menu/Select/Tooltip/DownloadQueue)。
```tsx
import { createPortal } from "react-dom";
// <Portal>{node}</Portal>
{createPortal(node, document.body)}
```
> 覆盖层组件多半可直接用 `@ark-ui/react` 的同名组件(Dialog/Menu/Tooltip/…)替换手写
> 版,已装。优先评估 ark 组件再决定是否手写 portal。

### createStore / produce（Toast）
`components/Toast.tsx` 用 `createStore`+`produce` 管全局 toast 数组。迁移:改成一个小 zustand
store(或就并入 `store.ts` 的模式),`produce` 的就地改写换成返回新数组的 setState。

---

## 2. Store 访问规则（**最重要,读两遍**)

`store.ts` 是一份 zustand store,导出了与 Solid 版**同名**的 getter/setter/action。两条读法:

- **组件内要响应式** → 用 hook,选择你需要的字段:
  ```tsx
  const page = useAppStore((s) => s.currentPage);
  const running = useAppStore((s) => s.runningIds.has(id));   // 派生也能选
  const list = useAppStore((s) => s.instances) ?? [];
  ```
- **非组件代码**(事件回调、工具函数、其它模块)→ 用 getter 取快照:
  ```tsx
  if (isRunning(id)) …            // = useAppStore.getState().runningIds.has(id)
  const root = activeRoot();
  ```

> 旧的 `currentPage()`/`instances()` getter **仍然存在**,但在组件里调它**不会**订阅、不会
> 重渲染。组件里一律改用 `useAppStore((s) => …)`。getter 只留给非组件代码。

setter/action 名字全部保留:`setCurrentPage`、`playInstance`、`refreshInstances`、
`kobeLogin`、`checkAllUpdates` … 直接 import 调用。
> ⚠️ zustand 的 setter 收**值**,不收 Solid 的 updater 函数。把 `setX(v => !v)` 改成
> `setX(!useAppStore.getState().x)`,或在组件里 `setX(!x)`。

---

## 3. i18n 规则

`t("ns.key", params?)` 签名、locale 词典文件**都不变**。唯一区别是「切语言即时重渲染」的
接线:
```tsx
export default function SomePanel() {
  useLang();                         // ← 订阅当前语言,切换时本组件重渲染
  return <h1>{t("settings.title")}</h1>;
}
```
- `App` 顶层已调一次 `useLang()`,覆盖**未 memo** 的整棵子树。
- 被 `React.memo` 隔离、或你希望语言切换时精确重渲染的组件,**自己在顶部调一次 `useLang()`**。
- 非组件代码直接 `t(...)` 取快照即可。
- 规则不变:改任何用户可见文案必须同步 `locales/` 的 **zh 和 en** 两侧,跑
  `node scripts/check-i18n.mjs`。**不要**新造词条却不落 locale(检查会红)。

---

## 4. 动画规则（`src/motion/react.tsx`)

迁移后只有 Toast 消费旧 motion 面。两个替换:
```tsx
// use:motion 指令 → useEntrance(ref 回调,不新增 DOM)
const ref = useEntrance("toast");           // 或 useEntrance("listStagger", { index: i })
return <div ref={ref} className="…">…</div>;

// 旧 <Presence>(从 children 空否推断) → 新 <Presence>(显式 show 布尔)
<Presence show={open} exitPreset="toast" onExited={() => remove(id)}>
  <ToastCard … />
</Presence>
```
两者都复用 `presets.ts` 的 WAAPI 关键帧,与 Classic「手感」零漂移,且都处理
`prefers-reduced-motion`。

**新写的复杂动画**(布局过渡/手势/编排)用再导出的 framer:`import { motion, AnimatePresence,
presetTransition } from "../motion/react"`。想对齐 Classic 时长/曲线时 `transition={presetTransition("dialog")}`。

`createTween` / `<Motion>` 无 app 消费者,已不提供。旧 `motion/solid.tsx`、`motion/engine.ts`
等留着不动(最后阶段删)。

---

## 5. keep-alive 布局(阶段③ 接 AppShell 时)

`layout/AppShell.tsx` 的常驻标签页:home/discover/library/agent/settings 首访后保持挂载,
切走只 `hidden`(display:none)不卸载,保留页内状态(搜索结果/滚动位置)。React 版保持同样
语义:常驻页始终渲染,用 `hidden` class 或 `style={{ display: active ? undefined : "none" }}`
切显隐,**不要**条件卸载它们(见记忆:remount-refetch bug 类)。实例详情(instance)按实例
重挂,不在常驻列表。

---

## 6. StrictMode 注意

`main.tsx` 开了 `<StrictMode>`:开发期 effect 会**跑两次**(挂载→清理→再挂载)。
- 副作用必须幂等 + 有清理(加监听要在 return 里移除;订阅要退订)。
- `store.ts` 的模块级副作用(轮询、事件监听)在模块求值时跑**一次**,不受 StrictMode 影响。

---

## 7. 分阶段文件清单(73 个 Solid 文件)

已完成(阶段①,勿重复):`store.ts`、`i18n.ts`、`motion/react.tsx`、`util/useAsync.ts`、
`main.tsx`、`App.tsx`、工具链。

**阶段② —— 无状态叶子组件 / primitives**(无或极少 store 依赖,先转好给后续复用):
`components/` 里:Button、Card、Checkbox、Chip、Icon、BlockIcon、InstanceIcon、Avatar、
Tag、Toggle、Segmented、Spinner、EmptyState、ErrorState、NavItem、Typography、Panel、
Badge/Tag 类、SearchBox、Slider、Tooltip、Menu、Select、Dialog(portal primitives)、
Toast(createStore + useEntrance/Presence 示范)。`components/index.ts`、`styles.ts`、
`format.ts` 为纯 TS,基本不动。

**阶段③ —— 布局壳**:`layout/AppShell.tsx`(keep-alive)、`Rail.tsx`、`TopBar.tsx`、
`ContextBar.tsx`;`routes.ts`(Component 类型换 React);`util/shortcuts.ts`、`gallery/runner.ts`。
把 `App.tsx` 占位换成 `<AppShell/>`。

**阶段④ —— 页面**:`pages/Home`、`Library`、`Discover`、`Settings`、`InstanceDetail`、
`ModpackDetail`、`ProjectInstallDetail`。

**阶段⑤ —— 重组件 + 对话框**:`InstanceManageDialog`、`RealmPanel`、`AccountDialog`、
`AccountMenu`、`NewInstanceDialog`、`ImportModpackDialog`、`ExportModpackDialog`、
`BlockedFilesDialog`、`JoinRealmDialog`、`SkinDialog`、`CrashDialog`、`ShortcutsHelp`、
`FriendsButton`、`FriendsSection`、`KobeAccountChip`、`LinkedAccountsSection`、
`NotificationCenter`、`ServersPanel`、`ProjectDetailPanel`、`ModpackOverview`、
`ModpackCard`、`ModpackListItem`、`InstanceRow`、`FacetSidebar`、`ContentBrowser`、
`DownloadQueue`、`Lightbox`、`PlayButton`;`util/downloads.ts`、`util/useModpackDrop.ts`。

**阶段⑥ —— agent**:`agent/ChatPage.tsx`(用 streamdown 渲染流式 markdown)、
`agent/chatStore.ts`(Solid 信号 parts → zustand;流式 reduce 逻辑照搬)、
`agent/markdownBlocks.ts`。

**阶段⑦ —— 收尾/移植**:`theme/theme.ts`(Solid createRoot/signal 管线 → zustand 或
useSyncExternalStore;公开签名 `initTheme`/`saveTheme`/`applyTheme` 不变)、`motion/reduced.ts`。
**待删**(需 owner 批准):`index.tsx`(旧入口)、`motion/solid.tsx`、`motion/engine.ts`、
`motion/targets.ts`、`motion/interpolators.ts`、以及确认无消费者后的 Solid 依赖。

---

## 8. 速查陷阱

- `class` → `className`;`for` → `htmlFor`;`onInput` → `onChange`(§1)。
- style 用对象:`style={{ marginTop: "8px" }}`(Solid 允许字符串;React 组件里优先对象,
  内联 CSS 变量如 `style={{ "--i": i } as React.CSSProperties}`)。
- 组件里读 store 用 `useAppStore((s)=>…)`,别调 `x()` getter(不订阅)。
- useEffect 必须手列 deps;Solid 的自动追踪没了。
- `<For>`/`.map` 必须给 `key`。
- StrictMode 下 effect 跑两次 —— 保持幂等 + 清理。
- 不要 remount 常驻页(用 hidden,不用条件卸载)。
- 不碰:`ipc/bindings.ts`(生成)、`locales/*.ts` 词条值、`theme/tokens.css`。
- 别删 Solid 文件 —— 删除集中在阶段⑦交 owner 批准。

### §8.1 Primitive 公开 API 裁决(Wave-1 定案)
- 所有 primitive 组件的公开样式 prop 一律 **`className`**(不留 Solid 的 `class`);消费方传 `className=`。
- 自定义值回调 prop 名(如 SearchBox/Slider 的 `onInput(v)`、`onCommit(v)`)**保持原名**——它们是普通函数 prop,与 React 语义无冲突,只有 DOM 属性形状的 prop(`class`/`for`/字符串 style)必须 React 化。
- `style` prop 只收 `React.CSSProperties` 对象;字符串 style 的调用点改对象。
- `Panel` 已删除 `classList` prop(零消费者):条件类在调用点用 `clsx(base, { … })` 拼进 `className`。
