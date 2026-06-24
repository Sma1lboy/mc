// 截图画廊驱动(开发用)。MC_GALLERY=1 启动时,App 挂载后调用 maybeRunGallery():
// 逐「镜头」驱动 UI(切 store 信号 / 点 DOM 触发器)→ 等渲染 → 调后端 gallery_capture
// 抓原生窗口 → 最后 gallery_build 生成 index.html 并打开。非画廊模式直接返回,零副作用。
//
// 截的是「真实 Tauri 窗口 + 真实数据」,不走 web/mock —— 与实际 app 完全一致。
// 镜头分两类:store 信号驱动(可靠)+ DOM 点击驱动(子 tab / 弹层 / 菜单,best-effort,
// 找不到触发器就记日志并跳过,不影响其余镜头)。

import { api } from "../ipc/api";
import {
  activeRoot,
  setCurrentPage,
  setDiscoverKind,
  setCurrentInstanceId,
} from "../store";

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/** 等两帧,确保信号变更后的 DOM 已经 paint 完。 */
const settle = (): Promise<void> =>
  new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));

/* ----------------------------------------------------------------------------
 * DOM 驱动小工具(运行在 webview 内,document 可用)。点不到就返回 false,调用方决定跳过。
 * -------------------------------------------------------------------------- */

const CLICKABLE = "button,[role=button],[role=tab],[role=radio],a";

function clickByAttr(attr: string, value: string): boolean {
  const el = document.querySelector(`[${attr}="${cssEscape(value)}"]`) as HTMLElement | null;
  if (el) {
    el.click();
    return true;
  }
  return false;
}

function clickByText(text: string): boolean {
  const els = Array.from(document.querySelectorAll(CLICKABLE)) as HTMLElement[];
  const el = els.find((e) => (e.textContent ?? "").trim() === text);
  if (el) {
    el.click();
    return true;
  }
  return false;
}

/** 关掉一切已打开的弹层 / 菜单(Esc + 外部点击),每个镜头开拍前都跑一次防止状态泄漏。
 * 关键:只在「确实有弹层打开」时才发 Esc——否则会误触如「实例详情 Esc 返回」等全局
 * keydown 监听,把后续子 tab 镜头踢回源页。Ark 的 Dialog/Menu/Popover 打开时其内容元素
 * 带 data-state="open",据此判断。尽力而为,绝不抛。 */
function closeOverlays(): void {
  try {
    const open = document.querySelector(
      '[role="dialog"][data-state="open"], [data-scope="menu"][data-state="open"], ' +
        '[data-scope="popover"][data-state="open"], [data-part="content"][data-state="open"]',
    );
    if (!open) return;
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    document.body.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  } catch {
    /* 关弹层尽力而为,出错不影响截图 */
  }
}

/** CSS 属性选择器值转义(标签/aria 文案里基本都是中文/英文,够用)。 */
function cssEscape(v: string): string {
  return v.replace(/["\\]/g, "\\$&");
}

/* ----------------------------------------------------------------------------
 * 镜头表
 * -------------------------------------------------------------------------- */

interface Shot {
  name: string;
  title: string;
  /** 驱动到目标状态;返回 false 表示跳过(无数据 / 触发器缺失)。 */
  setup: () => Promise<boolean | void> | boolean | void;
  /** setup 后额外等待毫秒(留给异步数据:搜索/列表/皮肤)。 */
  wait?: number;
}

// 实例镜头共享:首个实例就绪标志(由 instance-overview 置位,子 tab 镜头据此跳过)。
let instanceReady = false;

async function gotoFirstInstance(): Promise<boolean> {
  const list = await api.listInstances(activeRoot()).catch(() => []);
  if (!list.length) return false;
  setCurrentInstanceId(list[0].id);
  setCurrentPage("instance");
  instanceReady = true;
  return true;
}

/** 进发现页并切到某内容类型(切类型会整栏重挂 → 重搜,给足等待)。 */
function gotoDiscover(kind: "modpack" | "mod" | "shader" | "resourcepack" | "datapack"): void {
  setDiscoverKind(kind);
  setCurrentPage("discover");
}

const SHOTS: Shot[] = [
  { name: "home", title: "首页 Home", setup: () => setCurrentPage("home"), wait: 1800 },

  // ===== 发现:逐内容类型 + 源切换 =====
  { name: "discover-modpack", title: "发现 · 整合包", setup: () => gotoDiscover("modpack"), wait: 2800 },
  { name: "discover-mod", title: "发现 · 模组", setup: () => gotoDiscover("mod"), wait: 2600 },
  { name: "discover-shader", title: "发现 · 光影", setup: () => gotoDiscover("shader"), wait: 2600 },
  { name: "discover-resourcepack", title: "发现 · 资源包", setup: () => gotoDiscover("resourcepack"), wait: 2600 },
  { name: "discover-datapack", title: "发现 · 数据包", setup: () => gotoDiscover("datapack"), wait: 2600 },
  {
    name: "discover-curseforge",
    title: "发现 · CurseForge 源",
    setup: () => {
      gotoDiscover("modpack");
      // 切到 CurseForge 分段(可能因未配置 Key 而显示提示,也是有效一镜)。
      setTimeout(() => clickByText("CurseForge"), 400);
    },
    wait: 2800,
  },

  { name: "library", title: "库 Library", setup: () => setCurrentPage("library"), wait: 1400 },
  { name: "settings", title: "设置 Settings", setup: () => setCurrentPage("settings"), wait: 1100 },

  // ===== 实例详情:概览 + 各子 tab(DOM 点 tab 文案切换)=====
  { name: "instance-overview", title: "实例 · 概览", setup: () => gotoFirstInstance(), wait: 1600 },
  {
    name: "instance-settings",
    title: "实例 · 设置",
    setup: () => (instanceReady ? clickByText("设置") || true : false),
    wait: 900,
  },
  {
    name: "instance-mods",
    title: "实例 · Mods",
    setup: () => (instanceReady ? clickByText("Mods") || true : false),
    wait: 1400,
  },
  {
    name: "instance-resourcepack",
    title: "实例 · 资源包",
    setup: () => (instanceReady ? clickByText("资源包") || true : false),
    wait: 1400,
  },

  // ===== 弹层 / 菜单(DOM 点触发器;best-effort,点不到则跳过)=====
  {
    name: "menu-account",
    title: "账号切换菜单",
    setup: () => {
      setCurrentPage("home");
      return new Promise<boolean>((r) =>
        setTimeout(() => r(clickByAttr("aria-label", "切换账号")), 500),
      );
    },
    wait: 800,
  },
  {
    name: "dialog-new-instance",
    title: "新建实例对话框",
    setup: () => {
      setCurrentPage("home");
      return new Promise<boolean>((r) =>
        setTimeout(() => r(clickByAttr("title", "新增实例")), 500),
      );
    },
    wait: 900,
  },
  {
    name: "queue-downloads",
    title: "下载队列",
    setup: () => {
      setCurrentPage("home");
      return new Promise<boolean>((r) =>
        setTimeout(() => r(clickByAttr("aria-label", "下载队列")), 500),
      );
    },
    wait: 700,
  },
];

/** 画廊模式入口:非画廊模式即刻返回,否则跑完整截图流程。 */
export async function maybeRunGallery(): Promise<void> {
  let enabled = false;
  try {
    enabled = await api.galleryEnabled();
  } catch {
    return; // 旧二进制没有该命令 → 不是画廊模式
  }
  if (!enabled) return;

  // 等首屏数据(根目录、实例列表、主题)落定再开拍。
  await sleep(1800);

  const done: { name: string; title: string }[] = [];
  for (const shot of SHOTS) {
    try {
      closeOverlays(); // 开拍前清掉上一镜可能残留的弹层
      await settle();
      const ok = await shot.setup();
      if (ok === false) {
        console.warn(`[gallery] 跳过 ${shot.name}(无数据 / 触发器缺失)`);
        continue;
      }
      await settle();
      await sleep(shot.wait ?? 1200);
      await api.galleryCapture(shot.name);
      done.push({ name: shot.name, title: shot.title });
      console.log(`[gallery] 已截 ${shot.name}`);
    } catch (e) {
      console.error(`[gallery] ${shot.name} 截图失败`, e);
    }
  }
  closeOverlays();

  try {
    const index = await api.galleryBuild(done);
    console.log(`[gallery] 画廊已生成: ${index}`);
    // 用系统默认应用(浏览器)打开 index.html。
    await api.revealPath(index).catch(() => {});
  } catch (e) {
    console.error("[gallery] 生成画廊失败", e);
  }
}
