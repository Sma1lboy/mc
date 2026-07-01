import clsx from "clsx";

/**
 * Toggle —— 开关(替代裸 checkbox 的「启用/开」语义)。
 * 轨道 accent(开)/中性(关),圆钮滑动;role=switch 可达。一处定义,Mods/资源包/光影/数据包
 * 的「启用」与设置里的开关共用,观感统一。
 */
export function Toggle(props: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  title?: string;
}): React.ReactElement {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.checked}
      title={props.title}
      disabled={props.disabled}
      onClick={() => !props.disabled && props.onChange(!props.checked)}
      className={clsx(
        "relative inline-flex h-[24px] w-[44px] shrink-0 items-center p-[3px] rounded-none cursor-pointer shadow-input transition-colors duration-[var(--dur)] ease-app disabled:opacity-50 disabled:cursor-default",
        props.checked ? "bg-accent" : "bg-window",
      )}
    >
      <span
        className={clsx(
          "inline-block h-[18px] w-[18px] rounded-none shadow-raised transition-[transform,background-color] duration-[var(--dur)] ease-app",
          props.checked ? "translate-x-[20px] bg-white" : "translate-x-0 bg-[#6a6a5a]",
        )}
      />
    </button>
  );
}

export default Toggle;
