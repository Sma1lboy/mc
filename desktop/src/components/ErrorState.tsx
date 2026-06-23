import { Component, Show } from "solid-js";
import { t } from "../i18n";

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
    <div
      class={
        "flex flex-col items-center justify-center gap-[10px] rounded-card bg-glass-card border border-glass-border text-center " +
        (props.compact ? "px-[14px] py-[18px]" : "px-[24px] py-[28px]")
      }
    >
      <div class="text-[13px] text-danger-text leading-[1.6]">{props.message ?? t("components.errorState.failed")}</div>
      <Show when={props.onRetry}>
        <button
          class="h-[32px] px-[16px] rounded-ctl border border-glass-border bg-glass-card text-fg text-[13px] cursor-pointer transition-[background-color] duration-[var(--dur)] ease-app hover:bg-glass-hover"
          onClick={() => props.onRetry?.()}
        >
          {t("components.errorState.retry")}
        </button>
      </Show>
    </div>
  );
};

export default ErrorState;
