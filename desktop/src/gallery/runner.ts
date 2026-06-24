// 截图画廊驱动(开发用)。MC_GALLERY=1 启动时,App 挂载后调用 maybeRunGallery():
// 逐页切换 store 信号 → 等内容渲染 → 调后端 gallery_capture 抓原生窗口 → 最后
// gallery_build 生成 index.html 并用系统默认应用打开。非画廊模式直接返回,零副作用。
//
// 截的是「真实 Tauri 窗口 + 真实数据」,不走 web/mock —— 与实际 app 完全一致。

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

interface Shot {
  name: string;
  title: string;
  /** 切到该页的准备动作;返回 false 表示跳过(如无实例时的详情页)。 */
  setup: () => Promise<boolean | void> | boolean | void;
  /** setup 后额外等待毫秒数(留给异步数据:新闻/搜索/列表)。 */
  wait?: number;
}

const SHOTS: Shot[] = [
  {
    name: "home",
    title: "首页 Home",
    setup: () => setCurrentPage("home"),
    wait: 1800,
  },
  {
    name: "discover",
    title: "发现 Discover · 整合包",
    setup: () => {
      setDiscoverKind("modpack");
      setCurrentPage("discover");
    },
    wait: 2800,
  },
  {
    name: "library",
    title: "库 Library",
    setup: () => setCurrentPage("library"),
    wait: 1400,
  },
  {
    name: "settings",
    title: "设置 Settings",
    setup: () => setCurrentPage("settings"),
    wait: 1000,
  },
  {
    name: "instance",
    title: "实例详情 Instance",
    setup: async () => {
      const list = await api.listInstances(activeRoot()).catch(() => []);
      if (!list.length) return false; // 没有实例就跳过
      setCurrentInstanceId(list[0].id);
      setCurrentPage("instance");
    },
    wait: 1600,
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
      const ok = await shot.setup();
      if (ok === false) {
        console.warn(`[gallery] 跳过 ${shot.name}(无数据)`);
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

  try {
    const index = await api.galleryBuild(done);
    console.log(`[gallery] 画廊已生成: ${index}`);
    // 用系统默认应用(浏览器)打开 index.html。
    await api.revealPath(index).catch(() => {});
  } catch (e) {
    console.error("[gallery] 生成画廊失败", e);
  }
}
