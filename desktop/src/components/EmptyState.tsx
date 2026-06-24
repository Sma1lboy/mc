import { Component, JSX, Show } from "solid-js";
import { Panel } from "./Panel";

/**
 * EmptyState —— 统一的空状态卡片(列表为空 / 暂无内容)。
 * 一处定义,Home / Library / ContextBar / 搜索结果等共用,避免各页各写一套
 * (实线 vs 虚线、p-24 vs p-14、有边框 vs 无边框)导致观感参差。
 *
 * compact:右栏等窄容器用更小的内距。
 */
export const EmptyState: Component<{
  title: JSX.Element;
  hint?: JSX.Element;
  action?: JSX.Element;
  compact?: boolean;
}> = (props) => {
  return (
    <Panel
      variant="sunken"
      class={
        "flex flex-col items-center justify-center gap-[10px] text-center " +
        (props.compact ? "px-[14px] py-[18px]" : "px-[24px] py-[28px]")
      }
    >
      <div class="text-[13px] text-sub leading-[1.6]">{props.title}</div>
      <Show when={props.hint}>
        <div class="text-[12px] text-muted leading-[1.6]">{props.hint}</div>
      </Show>
      <Show when={props.action}>
        <div class="mt-[2px]">{props.action}</div>
      </Show>
    </Panel>
  );
};

export default EmptyState;
