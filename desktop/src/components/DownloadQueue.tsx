import { useSyncExternalStore } from "react";
import clsx from "clsx";
import { Popover } from "@ark-ui/react/popover";
import { Portal } from "@ark-ui/react/portal";
import { Icon } from "./Icon";
import {
  tasks,
  fractionOf,
  dismissDownload,
  clearFinished,
  type DownloadTask,
} from "../util/downloads";
import { t, useLang } from "../i18n";

/**
 * DownloadQueue —— 顶栏右上角的全局下载队列入口。
 * 一个下载图标按钮(进行中带角标计数),点开是一张面板,列出所有下载任务
 * (整合包 / mod / 光影 …):图标 + 标题 + 状态 + 进度条,已结束的可单条清除 / 一键清空。
 * 状态读 util/downloads 的全局队列,与 Discover 列表行的行内进度条同源。
 */

// ponytail: util/downloads.ts 仍是 Solid createSignal(阶段⑤ 才迁 zustand),没有框架无关的
// subscribe。迁移前用 200ms 轮询把 tasks() 快照喂给 React——setTasks 每次换新数组引用,
// useSyncExternalStore 的 Object.is 比对天然去重(引用没变不重渲染)。ceiling:进度条最多 5fps
// 刷新,够用;downloads.ts 迁到 zustand 后换它的 subscribe、删掉轮询。
function useTasks(): DownloadTask[] {
  return useSyncExternalStore((onChange) => {
    const h = setInterval(onChange, 200);
    return () => clearInterval(h);
  }, tasks);
}

function DownloadRow({ task }: { task: DownloadTask }) {
  useLang();
  const frac = fractionOf(task);
  const active = task.status === "active" || task.status === "queued";
  const statusLabel = (() => {
    switch (task.status) {
      case "queued":
        return t("downloads.queued");
      case "active":
        return task.stage || t("downloads.installing");
      case "done":
        return t("downloads.done");
      case "error":
        return task.error || t("downloads.failed");
    }
  })();

  return (
    <div className="flex items-center gap-[10px] px-[6px] py-[6px] rounded-none hover:bg-panel-2">
      <div className="shrink-0 w-[34px] h-[34px] rounded-none overflow-hidden bg-panel-2 shadow-input flex items-center justify-center">
        {task.icon ? (
          <img src={task.icon} alt="" className="w-full h-full object-cover" />
        ) : (
          <Icon name="download" size={16} className="text-dim" />
        )}
      </div>
      <div className="flex-1 min-w-0">
        <div
          className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis"
          title={task.title}
        >
          {task.title}
        </div>
        <div
          className={clsx("text-[11px] whitespace-nowrap overflow-hidden text-ellipsis", {
            "text-danger-text": task.status === "error",
            "text-accent": task.status === "done",
            "text-dim": active,
          })}
          title={statusLabel}
        >
          {statusLabel}
        </div>
        {active && (
          // 单条稳定进度条:用 class 在「流动(total 未知)」与「定量」间切换,不换 DOM
          // 元素——避免 total 在阶段切换间瞬时归 0 时反复重建元素导致的闪烁/消失。
          <div className="mt-[5px] h-[5px] rounded-none bg-panel-2 shadow-input overflow-hidden">
            <div
              className={clsx("h-full bg-accent", {
                "w-1/3 [animation:dl-indeterminate_1.1s_ease-in-out_infinite]": frac === null,
                "transition-[width] duration-200 ease-app": frac !== null,
              })}
              style={frac !== null ? { width: `${Math.round((frac ?? 0) * 100)}%` } : undefined}
            />
          </div>
        )}
      </div>
      {!active && (
        <button
          type="button"
          className="shrink-0 w-[22px] h-[22px] grid place-items-center rounded-none border-none bg-transparent text-dim cursor-pointer hover:text-fg hover:bg-panel-2"
          aria-label={t("downloads.dismiss")}
          onClick={() => dismissDownload(task.id)}
        >
          <Icon name="close" size={12} />
        </button>
      )}
    </div>
  );
}

export function DownloadQueue() {
  useLang();
  const items = useTasks();
  const count = items.filter((task) => task.status === "queued" || task.status === "active").length;
  const hasFinished = items.some((task) => task.status === "done" || task.status === "error");

  return (
    <Popover.Root positioning={{ placement: "bottom-end", gutter: 8 }}>
      <Popover.Trigger
        className="relative inline-flex items-center justify-center w-[30px] h-[30px] rounded-none border-none bg-transparent text-dim cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-panel-2 hover:text-fg data-[state=open]:bg-panel-2 data-[state=open]:text-fg [-webkit-app-region:no-drag]"
        aria-label={t("downloads.title")}
      >
        <Icon name="download" size={16} />
        {count > 0 && (
          <span className="absolute -top-[3px] -right-[3px] min-w-[15px] h-[15px] px-[3px] rounded-none bg-accent shadow-raised text-white text-[10px] leading-[15px] font-semibold text-center">
            {count}
          </span>
        )}
      </Popover.Trigger>
      <Portal>
        <Popover.Positioner>
          <Popover.Content className="z-[200] w-[320px] max-h-[440px] overflow-y-auto bg-panel-2 shadow-raised border border-titlebar rounded-none p-[10px] [-webkit-app-region:no-drag] focus:outline-none">
            <div className="flex items-center justify-between px-[6px] pt-[2px] pb-[6px]">
              <span className="text-[13px] font-semibold text-fg">{t("downloads.title")}</span>
              {hasFinished && (
                <button
                  type="button"
                  className="text-[12px] text-dim hover:text-fg cursor-pointer bg-transparent border-none"
                  onClick={() => clearFinished()}
                >
                  {t("downloads.clearFinished")}
                </button>
              )}
            </div>
            {items.length > 0 ? (
              <div className="flex flex-col gap-[4px]">
                {items.map((task) => (
                  <DownloadRow key={task.id} task={task} />
                ))}
              </div>
            ) : (
              <div className="px-[6px] py-[22px] text-center text-[12px] text-dim">
                {t("downloads.empty")}
              </div>
            )}
          </Popover.Content>
        </Popover.Positioner>
      </Portal>
    </Popover.Root>
  );
}

export default DownloadQueue;
