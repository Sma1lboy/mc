import { Component, For, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Popover } from "@ark-ui/solid/popover";
import { Icon } from "./Icon";
import {
  tasks,
  inflightCount,
  fractionOf,
  dismissDownload,
  clearFinished,
  type DownloadTask,
} from "../util/downloads";
import { t } from "../i18n";

/**
 * DownloadQueue —— 顶栏右上角的全局下载队列入口。
 * 一个下载图标按钮(进行中带角标计数),点开是一张面板,列出所有下载任务
 * (整合包 / mod / 光影 …):图标 + 标题 + 状态 + 进度条,已结束的可单条清除 / 一键清空。
 * 状态读 util/downloads 的全局队列,与 Discover 列表行的行内进度条同源。
 */

const DownloadRow: Component<{ task: DownloadTask }> = (props) => {
  const frac = () => fractionOf(props.task);
  const active = () => props.task.status === "active" || props.task.status === "queued";
  const statusLabel = () => {
    switch (props.task.status) {
      case "queued":
        return t("downloads.queued");
      case "active":
        return props.task.stage || t("downloads.installing");
      case "done":
        return t("downloads.done");
      case "error":
        return props.task.error || t("downloads.failed");
    }
  };

  return (
    <div class="flex items-center gap-[10px] px-[6px] py-[6px] rounded-ctl hover:bg-glass-hover">
      <div class="shrink-0 w-[34px] h-[34px] rounded-[8px] overflow-hidden bg-glass-card flex items-center justify-center">
        <Show when={props.task.icon} fallback={<Icon name="download" size={16} class="text-dim" />}>
          <img src={props.task.icon!} alt="" class="w-full h-full object-cover" />
        </Show>
      </div>
      <div class="flex-1 min-w-0">
        <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis" title={props.task.title}>
          {props.task.title}
        </div>
        <div
          class="text-[11px] whitespace-nowrap overflow-hidden text-ellipsis"
          classList={{
            "text-danger-text": props.task.status === "error",
            "text-a-6": props.task.status === "done",
            "text-dim": active(),
          }}
          title={statusLabel()}
        >
          {statusLabel()}
        </div>
        <Show when={active()}>
          <div class="mt-[5px] h-[3px] rounded-full bg-glass-card overflow-hidden">
            <Show
              when={frac() !== null}
              fallback={<div class="h-full w-1/3 bg-a-5 [animation:dl-indeterminate_1.1s_ease-in-out_infinite]" />}
            >
              <div
                class="h-full bg-a-5 transition-[width] duration-200 ease-app"
                style={{ width: `${Math.round((frac() ?? 0) * 100)}%` }}
              />
            </Show>
          </div>
        </Show>
      </div>
      <Show when={!active()}>
        <button
          type="button"
          class="shrink-0 w-[22px] h-[22px] grid place-items-center rounded-ctl border-none bg-transparent text-dim cursor-pointer hover:text-fg hover:bg-glass-hover"
          aria-label={t("downloads.dismiss")}
          onClick={() => dismissDownload(props.task.id)}
        >
          <Icon name="close" size={12} />
        </button>
      </Show>
    </div>
  );
};

export const DownloadQueue: Component = () => {
  const items = tasks;
  const count = inflightCount;
  const hasFinished = () => items().some((task) => task.status === "done" || task.status === "error");

  return (
    <Popover.Root positioning={{ placement: "bottom-end", gutter: 8 }}>
      <Popover.Trigger
        class="relative inline-flex items-center justify-center w-[30px] h-[30px] rounded-ctl border-none bg-transparent text-dim cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-glass-hover hover:text-fg data-[state=open]:bg-glass-hover data-[state=open]:text-fg [-webkit-app-region:no-drag]"
        aria-label={t("downloads.title")}
      >
        <Icon name="download" size={16} />
        <Show when={count() > 0}>
          <span class="absolute -top-[3px] -right-[3px] min-w-[15px] h-[15px] px-[3px] rounded-full bg-a-5 text-white text-[10px] leading-[15px] font-semibold text-center">
            {count()}
          </span>
        </Show>
      </Popover.Trigger>
      <Portal>
        <Popover.Positioner>
          <Popover.Content class="z-[200] w-[320px] max-h-[440px] overflow-y-auto glass-pop rounded-card p-[10px] [-webkit-app-region:no-drag] focus:outline-none">
            <div class="flex items-center justify-between px-[6px] pt-[2px] pb-[6px]">
              <span class="text-[13px] font-semibold text-fg">{t("downloads.title")}</span>
              <Show when={hasFinished()}>
                <button
                  type="button"
                  class="text-[12px] text-dim hover:text-fg cursor-pointer bg-transparent border-none"
                  onClick={() => clearFinished()}
                >
                  {t("downloads.clearFinished")}
                </button>
              </Show>
            </div>
            <Show
              when={items().length > 0}
              fallback={<div class="px-[6px] py-[22px] text-center text-[12px] text-dim">{t("downloads.empty")}</div>}
            >
              <div class="flex flex-col gap-[4px]">
                <For each={items()}>{(task) => <DownloadRow task={task} />}</For>
              </div>
            </Show>
          </Popover.Content>
        </Popover.Positioner>
      </Portal>
    </Popover.Root>
  );
};

export default DownloadQueue;
