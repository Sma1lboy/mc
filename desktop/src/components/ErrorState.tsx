import { Component, Show } from "solid-js";
import { t } from "../i18n";
import { Panel } from "./Panel";
import { Button } from "./Button";

/**
 * ErrorState —— 统一的「加载失败」占位(与 EmptyState 同形,但 danger 文案 + 重试)。
 * 一处定义,各 createResource 失败时共用,避免把「加载失败」误显示成「还没有…」的空态。
 *
 * compact:右栏 / 标签页等窄容器用更小内距。
 */
export const ErrorState: Component<{
  message?: string;
  onRetry?: () => void;
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
      <div class="text-[13px] text-danger-text leading-[1.6]">{props.message ?? t("components.errorState.failed")}</div>
      <Show when={props.onRetry}>
        <Button variant="ghost" onClick={() => props.onRetry?.()}>
          {t("components.errorState.retry")}
        </Button>
      </Show>
    </Panel>
  );
};

export default ErrorState;
