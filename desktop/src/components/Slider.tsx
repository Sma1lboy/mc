import type { ReactNode } from "react";
import "./Slider.css";

/* ============================================================================
 * Slider —— Blocky Craft 滑块(内存 / 并发 / 外观四项)。
 * 深凹轨 + 方块橙滑钮(见 Slider.css)。可选顶部标签行:左标题、右实时值
 * (点阵体)。值由 display() 自定义(如 2048 → "2G")。
 * ========================================================================== */

export interface SliderProps {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onInput: (value: number) => void;
  /** 提交(松手 / 键盘 change)时触发,用于持久化;拖动中的实时应用走 onInput,避免每
   * 一个 input tick 都落盘(见 Settings 主题/内存/并发滑块)。省略则不区分,仅 onInput。 */
  onCommit?: (value: number) => void;
  /** 顶部标签行的左侧标题;提供才渲染标签行。 */
  label?: ReactNode;
  /** 实时值显示;默认显示原始数值。 */
  display?: (value: number) => string;
  disabled?: boolean;
  /** range 的无障碍标签(无 label 文案时用)。 */
  ariaLabel?: string;
  className?: string;
}

export function Slider(props: SliderProps): React.ReactElement {
  const shown = props.display ? props.display(props.value) : String(props.value);
  // 提交语义:Solid 版靠原生 change(松手 / 键盘步进)持久化。React onChange 是 input
  // 语义(每 tick),故 commit 改由指针松手 + 键盘抬起驱动,读当前 range 值上抛。
  const commit = (el: HTMLInputElement): void => props.onCommit?.(Number(el.value));
  return (
    <div className={props.className}>
      {props.label !== undefined && (
        <div className="flex justify-between items-baseline mb-[8px]">
          <span className="text-[12px] text-sub">{props.label}</span>
          <span className="font-pixel text-[10px] text-fg">{shown}</span>
        </div>
      )}
      <input
        type="range"
        className="kb-range"
        min={props.min ?? 0}
        max={props.max ?? 100}
        step={props.step ?? 1}
        value={props.value}
        disabled={props.disabled}
        aria-label={props.ariaLabel}
        onChange={(e) => props.onInput(Number(e.currentTarget.value))}
        onPointerUp={(e) => commit(e.currentTarget)}
        onKeyUp={(e) => commit(e.currentTarget)}
      />
    </div>
  );
}

export default Slider;
