import { JSX, Show } from "solid-js";
import { PlayButton } from "./PlayButton";
import { formatRelativeTime } from "./format";

// InstanceRow 接收的实例形状。与后端 InstanceSummary 字段对齐
// (id,name,mc_version,loader,loader_version,icon,last_played,running)。
// 这里只声明本组件用到的字段, loader_version / icon 可选。
export interface InstanceRowData {
  id: string;
  name: string;
  mc_version: string;
  loader: string;
  loader_version?: string;
  icon?: string;
  last_played: number; // epoch ms
  running: boolean;
}

// InstanceRow —— "Jump back in" 横行卡。
// props 契约:
//   instance: 实例数据
//   onPlay?: 点 Play 回调 (传入实例 id)
//   onMenu?: 点 ⋮ 菜单回调 (传入实例 id 与点击事件, 供页面定位弹出菜单)
export interface InstanceRowProps {
  instance: InstanceRowData;
  onPlay?: (id: string) => void;
  onMenu?: (id: string, e: MouseEvent) => void;
}

export function InstanceRow(props: InstanceRowProps): JSX.Element {
  const inst = () => props.instance;

  // 名称首字母 (图标占位)。
  const initial = () => {
    const n = inst().name?.trim();
    return n && n.length > 0 ? n[0] : "?";
  };

  // 元信息行: "Fabric 1.20.1 · Played 5 minutes ago"。
  // loader 首字母大写, 拼 mc_version; last_played 走相对时间格式化。
  const loaderLabel = () => {
    const l = inst().loader;
    if (!l) return inst().mc_version;
    const cap = l.charAt(0).toUpperCase() + l.slice(1);
    return `${cap} ${inst().mc_version}`;
  };

  const playedLabel = () => {
    const rel = formatRelativeTime(inst().last_played);
    // "never" 时显示 "Never played", 否则 "Played x ago"。
    return rel === "never" ? "Never played" : `Played ${rel}`;
  };

  return (
    <div class="flex items-center gap-[14px] bg-card rounded-card shadow-card border border-transparent px-[14px] py-[12px] transition-[transform,box-shadow,border-color,background-color] duration-[var(--dur)] ease-app hover:-translate-y-[2px] hover:shadow-[0_6px_20px_rgba(0,0,0,0.42)] hover:border-n-6">
      {/* 左: 图标 (有 icon 显示图片, 否则渐变 + 首字母)。 */}
      <div class="relative shrink-0 w-[48px] h-[48px] rounded-ctl overflow-hidden flex items-center justify-center bg-gradient-to-br from-a-3 to-a-5 text-white font-bold text-[20px] uppercase select-none">
        <Show when={inst().icon} fallback={<span>{initial()}</span>}>
          <img src={inst().icon} alt="" loading="lazy" class="w-full h-full object-cover block" />
        </Show>
        {/* 运行中绿点。 */}
        <Show when={inst().running}>
          <span
            class="absolute right-[2px] bottom-[2px] w-[11px] h-[11px] rounded-full bg-a-5 shadow-[0_0_0_2px_var(--bg-card)]"
            title="Running"
          />
        </Show>
      </div>

      {/* 中: 名称 + 元信息。 */}
      <div class="flex-1 min-w-0 flex flex-col gap-[3px]">
        <div
          class="text-[length:var(--fs-base)] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis"
          title={inst().name}
        >
          {inst().name}
        </div>
        <div class="text-[12px] text-dim whitespace-nowrap overflow-hidden text-ellipsis flex items-center gap-[6px]">
          <span>{loaderLabel()}</span>
          <span class="opacity-50">·</span>
          <span>{playedLabel()}</span>
        </div>
      </div>

      {/* 右: Play + ⋮ 菜单。 */}
      <div class="shrink-0 flex items-center gap-[6px]">
        <PlayButton
          running={inst().running}
          onClick={() => props.onPlay?.(inst().id)}
        />
        <button
          type="button"
          class="inline-flex items-center justify-center w-[34px] h-[34px] border-none bg-transparent text-dim rounded-ctl cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-n-5 hover:text-fg"
          aria-label="More options"
          onClick={(e) => props.onMenu?.(inst().id, e)}
        >
          {/* ⋮ 竖向三点。 */}
          <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <circle cx="8" cy="3" r="1.5" />
            <circle cx="8" cy="8" r="1.5" />
            <circle cx="8" cy="13" r="1.5" />
          </svg>
        </button>
      </div>
    </div>
  );
}

export default InstanceRow;
