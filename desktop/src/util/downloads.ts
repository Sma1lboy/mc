// 下载队列 —— 全局下载管理器。
// install://progress 是单一全局事件流(无任务 id),所以这里把下载**串行化**(同一时刻
// 只跑一个活跃任务),并把这条流路由给当前活跃任务;副产物正好是一个真实的「下载队列」。
// 任意页面用 enqueueDownload(...) 投递任务;顶栏队列面板与列表行直接读 tasks() 渲染进度。
import { create } from "zustand";
import { onInstallProgress } from "../ipc/api";
import type { ProjectKind } from "../ipc/types";

export type DownloadStatus = "queued" | "active" | "done" | "error";
/** 下载资源类型:整合包 / mod / 光影 等,或裸版本(version)。 */
export type DownloadKind = ProjectKind | "version";

/** 队列里一条下载任务的可展示视图(响应式,immutably 更新)。 */
export interface DownloadTask {
  readonly id: string;
  /** 关联的项目 / 资源 id,供列表行就地查询进度(可空)。 */
  readonly refId?: string;
  readonly title: string;
  readonly icon?: string | null;
  readonly kind: DownloadKind;
  status: DownloadStatus;
  stage: string;
  current: number;
  total: number;
  speedBps: number;
  error?: string;
}

/** 投递任务的入参。run 为真正的安装动作;完成 / 失败通过回调通知调用方。 */
export interface EnqueueArgs {
  refId?: string;
  title: string;
  icon?: string | null;
  kind: DownloadKind;
  run: () => Promise<unknown>;
  onComplete?: (result: unknown) => void;
  onError?: (err: unknown) => void;
}

interface Job {
  run: () => Promise<unknown>;
  onComplete?: (result: unknown) => void;
  onError?: (err: unknown) => void;
}

// 队列视图存进 zustand:组件用 useDownloadStore((s) => s.tasks) 真订阅;非组件代码走
// tasks() 取快照。exported 名字与旧 Solid 版一致(tasks 仍是取值 getter)。
interface DownloadState {
  tasks: DownloadTask[];
}
export const useDownloadStore = create<DownloadState>(() => ({ tasks: [] }));

/** 当前队列快照(非组件代码用;组件请用 useTasks / useDownloadStore 订阅)。 */
export const tasks = (): DownloadTask[] => useDownloadStore.getState().tasks;

/** 组件里订阅整条队列(切换/进度变化即重渲染)。 */
export const useTasks = (): DownloadTask[] => useDownloadStore((s) => s.tasks);

function setTasks(updater: (ts: DownloadTask[]) => DownloadTask[]): void {
  useDownloadStore.setState((s) => ({ tasks: updater(s.tasks) }));
}

// 任务的副作用(run / 回调)与可展示视图分开存:视图响应式,Job 留在 Map 里不入信号。
const jobs = new Map<string, Job>();
let seq = 0;
let activeId: string | null = null;
let pumping = false;
let subscribed = false;

function patch(id: string, partial: Partial<DownloadTask>) {
  setTasks((ts) => ts.map((task) => (task.id === id ? { ...task, ...partial } : task)));
}

// 单一全局订阅:把共享的 install://progress 流路由给当前活跃任务(串行化保证不会串台)。
function ensureSubscribed() {
  if (subscribed) return;
  subscribed = true;
  onInstallProgress((p) => {
    if (activeId) patch(activeId, { stage: p.stage, current: p.current, total: p.total, speedBps: p.speed_bps });
  });
}

// 队列泵:一次只跑一个 queued 任务,跑完再取下一个;运行中再投递的任务会被下一轮取到。
async function pump() {
  if (pumping) return;
  pumping = true;
  try {
    for (;;) {
      const next = tasks().find((task) => task.status === "queued");
      if (!next) break;
      activeId = next.id;
      patch(next.id, { status: "active", stage: "", current: 0, total: 0 });
      const job = jobs.get(next.id);
      try {
        const result = await job?.run();
        patch(next.id, { status: "done", current: 1, total: 1 });
        job?.onComplete?.(result);
      } catch (e) {
        patch(next.id, { status: "error", error: String(e) });
        job?.onError?.(e);
      } finally {
        activeId = null;
        jobs.delete(next.id);
      }
    }
  } finally {
    pumping = false;
  }
}

/** 投递一条下载,返回任务 id。立即入队并尝试推进队列。 */
export function enqueueDownload(args: EnqueueArgs): string {
  ensureSubscribed();
  const id = `dl-${++seq}`;
  jobs.set(id, { run: args.run, onComplete: args.onComplete, onError: args.onError });
  setTasks((ts) => [
    ...ts,
    {
      id,
      refId: args.refId,
      title: args.title,
      icon: args.icon ?? null,
      kind: args.kind,
      status: "queued",
      stage: "",
      current: 0,
      total: 0,
      speedBps: 0,
    },
  ]);
  void pump();
  return id;
}

/** 某资源 id 当前的下载任务(优先进行中,否则取最近一条),供列表行显示进度 / 已添加。 */
export function downloadForRef(refId: string): DownloadTask | undefined {
  const ts = tasks();
  return (
    ts.find((task) => task.refId === refId && (task.status === "active" || task.status === "queued")) ??
    [...ts].reverse().find((task) => task.refId === refId)
  );
}

/** 进行中(排队 + 活跃)的任务数,顶栏角标用。 */
export const inflightCount = () => tasks().filter((t) => t.status === "queued" || t.status === "active").length;

/** 0..1 的确定进度;total 为 0(未知)返回 null —— UI 应显示流动条而非定量进度。 */
export function fractionOf(task: DownloadTask): number | null {
  if (task.status === "done") return 1;
  if (task.total <= 0) return null;
  return Math.min(1, task.current / task.total);
}

/** 从队列移除一条(完成 / 失败后用户手动清理)。 */
export function dismissDownload(id: string) {
  jobs.delete(id);
  setTasks((ts) => ts.filter((t) => t.id !== id));
}

/** 清掉所有已结束(done / error)的任务。 */
export function clearFinished() {
  setTasks((ts) => ts.filter((t) => t.status === "queued" || t.status === "active"));
}
