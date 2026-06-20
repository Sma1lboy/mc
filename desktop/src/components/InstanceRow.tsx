import { JSX, Show, createSignal } from "solid-js";
import { PlayButton } from "./PlayButton";
import { Dialog } from "./Dialog";
import { Menu } from "./Menu";
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
//   onPlay?/onOpenDir?/onExport?/onDelete?: ⋮ 上下文菜单各动作回调(传入实例 id)。
//   删除前本组件内置确认弹窗,确认后才触发 onDelete。
export interface InstanceRowProps {
  instance: InstanceRowData;
  onPlay?: (id: string) => void;
  onOpenDir?: (id: string) => void;
  onExport?: (id: string) => void;
  onDelete?: (id: string) => void;
}

export function InstanceRow(props: InstanceRowProps): JSX.Element {
  const inst = () => props.instance;
  const [confirmOpen, setConfirmOpen] = createSignal(false);

  // 名称首字母 (图标占位)。
  const initial = () => {
    const n = inst().name?.trim();
    return n && n.length > 0 ? n[0] : "?";
  };

  // 元信息行: "Fabric 1.20.1 · Played 5 minutes ago"。
  const loaderLabel = () => {
    const l = inst().loader;
    if (!l) return inst().mc_version;
    const cap = l.charAt(0).toUpperCase() + l.slice(1);
    return `${cap} ${inst().mc_version}`;
  };

  const playedLabel = () => {
    const rel = formatRelativeTime(inst().last_played);
    return rel === "never" ? "Never played" : `Played ${rel}`;
  };

  const onSelectAction = (value: string) => {
    const id = inst().id;
    if (value === "play") props.onPlay?.(id);
    else if (value === "open") props.onOpenDir?.(id);
    else if (value === "export") props.onExport?.(id);
    else if (value === "delete") setConfirmOpen(true);
  };

  return (
    <>
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

        {/* 右: Play + ⋮ 菜单(Ark Menu:键盘可达 + 点外部/Esc 关闭)。 */}
        <div class="shrink-0 flex items-center gap-[6px]">
          <PlayButton running={inst().running} onClick={() => props.onPlay?.(inst().id)} />
          <Menu.Root positioning={{ placement: "bottom-end" }} onSelect={(d: { value: string }) => onSelectAction(d.value)}>
            <Menu.Trigger
              class="inline-flex items-center justify-center w-[34px] h-[34px] border-none bg-transparent text-dim rounded-ctl cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-n-5 hover:text-fg data-[state=open]:bg-n-5 data-[state=open]:text-fg"
              aria-label="更多操作"
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
                <circle cx="8" cy="3" r="1.5" />
                <circle cx="8" cy="8" r="1.5" />
                <circle cx="8" cy="13" r="1.5" />
              </svg>
            </Menu.Trigger>
            <Menu.Content>
              <Menu.Item value="play">启动</Menu.Item>
              <Menu.Item value="open">打开游戏目录</Menu.Item>
              <Menu.Item value="export">导出整合包(.mrpack)</Menu.Item>
              <Menu.Separator />
              <Menu.Item value="delete" danger>
                删除实例
              </Menu.Item>
            </Menu.Content>
          </Menu.Root>
        </div>
      </div>

      {/* 删除确认弹窗(Ark Dialog) */}
      <Dialog
        open={confirmOpen()}
        onClose={() => setConfirmOpen(false)}
        label="删除实例"
        contentClass="w-[360px] max-w-[calc(100vw-48px)] bg-card rounded-card shadow-card overflow-hidden focus:outline-none"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-fg">删除实例「{inst().name}」?</div>
          <div class="text-[13px] text-dim leading-[1.6]">
            将永久删除该版本目录,包括其 mods、存档与配置。此操作不可撤销。
          </div>
          <div class="flex justify-end gap-[10px]">
            <button
              class="h-[34px] px-[16px] border border-n-6 rounded-xs bg-n-4 text-fg text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-n-5"
              onClick={() => setConfirmOpen(false)}
            >
              取消
            </button>
            <button
              class="h-[34px] px-[16px] border-none rounded-xs bg-[#d9534f] text-white text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-[#c44]"
              onClick={() => {
                setConfirmOpen(false);
                props.onDelete?.(inst().id);
              }}
            >
              删除
            </button>
          </div>
        </div>
      </Dialog>
    </>
  );
}

export default InstanceRow;
