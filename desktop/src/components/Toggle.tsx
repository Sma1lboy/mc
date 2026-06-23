import { Component } from "solid-js";

/**
 * Toggle —— 开关(替代裸 checkbox 的「启用/开」语义)。
 * 轨道 accent(开)/中性(关),圆钮滑动;role=switch 可达。一处定义,Mods/资源包/光影/数据包
 * 的「启用」与设置里的开关共用,观感统一。
 */
export const Toggle: Component<{
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  title?: string;
}> = (props) => {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.checked}
      title={props.title}
      disabled={props.disabled}
      onClick={() => !props.disabled && props.onChange(!props.checked)}
      class="relative inline-flex h-[18px] w-[32px] shrink-0 items-center rounded-full cursor-pointer transition-colors duration-[var(--dur)] ease-app disabled:opacity-50 disabled:cursor-default"
      classList={{ "bg-a-4": props.checked, "bg-n-6": !props.checked }}
    >
      <span
        class="inline-block h-[14px] w-[14px] rounded-full bg-white transition-transform duration-[var(--dur)] ease-app"
        classList={{ "translate-x-[16px]": props.checked, "translate-x-[2px]": !props.checked }}
      />
    </button>
  );
};

export default Toggle;
