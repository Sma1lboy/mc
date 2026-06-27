import { Component, Show } from "solid-js";

/**
 * Checkbox —— 方块复选框 + 标签(方块工坊风:选中 accent 填充 + 勾,未选中 shadow-input 凹陷)。
 * 与 FacetSidebar 的多选项观感统一;裸 <input type="checkbox"> 的浏览器默认样式不再使用。
 */
export const Checkbox: Component<{
  checked: boolean;
  onChange: (next: boolean) => void;
  label: string;
  disabled?: boolean;
}> = (props) => (
  <button
    type="button"
    role="checkbox"
    aria-checked={props.checked}
    disabled={props.disabled}
    onClick={() => !props.disabled && props.onChange(!props.checked)}
    class="inline-flex items-center gap-[7px] rounded-none border-none bg-transparent p-0 cursor-pointer text-[12px] transition-colors duration-[var(--dur)] ease-app disabled:opacity-50 disabled:cursor-default focus-visible:outline-none"
    classList={{ "text-fg": props.checked, "text-muted": !props.checked }}
  >
    <span
      class="shrink-0 inline-flex items-center justify-center w-[16px] h-[16px] rounded-none transition-[background-color] duration-[var(--dur)] ease-app"
      classList={{
        "bg-accent text-accent-text shadow-raised": props.checked,
        "bg-panel-2 shadow-input": !props.checked,
      }}
    >
      <Show when={props.checked}>
        <svg width="11" height="11" viewBox="0 0 12 12" fill="none" aria-hidden="true">
          <path d="m2.5 6.2 2.3 2.3L9.5 3.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </Show>
    </span>
    <span class="select-none">{props.label}</span>
  </button>
);

export default Checkbox;
