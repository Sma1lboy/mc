import { Component, For, createMemo } from "solid-js";
import { Portal } from "solid-js/web";
import { Select as Ark, createListCollection } from "@ark-ui/solid/select";
import { Check, ChevronDown } from "lucide-solid";

/**
 * Select —— 基于 Ark UI(headless)的house-styled 下拉选择,替代原生 <select>。
 * 全部用 Tailwind 工具类 + 我们的令牌着色,a11y/键盘/定位由 Ark 负责。
 * 单选:value 是字符串,options 形如 {label, value}。
 */
export interface SelectOption {
  label: string;
  value: string;
}

export interface SelectProps {
  value: string;
  onChange: (value: string) => void;
  options: SelectOption[];
  placeholder?: string;
  /** 触发器额外类名(宽度等)。 */
  class?: string;
}

export const Select: Component<SelectProps> = (props) => {
  // 响应式 collection:options 变化时重建。
  const collection = createMemo(() => createListCollection({ items: props.options }));

  return (
    <Ark.Root
      collection={collection()}
      value={[props.value]}
      onValueChange={(d: { value: string[] }) => props.onChange(d.value[0] ?? "")}
      positioning={{ sameWidth: true, placement: "bottom" }}
    >
      <Ark.Control>
        <Ark.Trigger
          class={
            "inline-flex items-center justify-between gap-[8px] min-w-[200px] px-[10px] py-[6px] " +
            "rounded-ctl glass-panel border border-glass-border text-fg text-[13px] cursor-pointer " +
            "transition-[border-color,box-shadow] duration-150 ease-app hover:border-a-4 data-[state=open]:border-a-4 " +
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5/25 " +
            (props.class ?? "")
          }
        >
          <Ark.ValueText placeholder={props.placeholder ?? "请选择"} class="truncate text-left" />
          <Ark.Indicator class="shrink-0 text-dim transition-transform duration-150 data-[state=open]:rotate-180">
            <ChevronDown size={16} />
          </Ark.Indicator>
        </Ark.Trigger>
      </Ark.Control>
      <Portal>
        <Ark.Positioner>
          <Ark.Content class="z-[300] max-h-[320px] overflow-y-auto rounded-ctl glass-pop p-[4px] text-[13px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5">
            <For each={props.options}>
              {(opt) => (
                <Ark.Item
                  item={opt}
                  class="flex items-center justify-between gap-[8px] px-[10px] py-[7px] rounded-xs text-fg cursor-pointer select-none data-[highlighted]:bg-glass-hover data-[state=checked]:text-a-6"
                >
                  <Ark.ItemText class="truncate">{opt.label}</Ark.ItemText>
                  <Ark.ItemIndicator class="shrink-0 text-a-6">
                    <Check size={15} />
                  </Ark.ItemIndicator>
                </Ark.Item>
              )}
            </For>
          </Ark.Content>
        </Ark.Positioner>
      </Portal>
    </Ark.Root>
  );
};

export default Select;
