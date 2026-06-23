import { JSX, Show, createSignal } from "solid-js";
import { PlayButton } from "./PlayButton";
import { Dialog } from "./Dialog";
import { Menu } from "./Menu";
import { formatRelativeTime } from "./format";
import { loaderLabel as fmtLoader } from "../util/loaders";
import { isRunning, isLaunching } from "../store";
import { t } from "../i18n";

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
  /** 点击行主体(图标+名称区)进入实例详情。 */
  onOpen?: (id: string) => void;
  onManage?: (id: string) => void;
  onOpenDir?: (id: string) => void;
  onExport?: (id: string) => void;
  onDelete?: (id: string) => void;
}

export function InstanceRow(props: InstanceRowProps): JSX.Element {
  const inst = () => props.instance;
  const [confirmOpen, setConfirmOpen] = createSignal(false);
  // 运行态以进程注册表为准(后端 game://started/exit 实时同步),而非静态的 instance.running。
  const running = () => isRunning(inst().id);

  // 名称首字母 (图标占位)。
  const initial = () => {
    const n = inst().name?.trim();
    return n && n.length > 0 ? n[0] : "?";
  };

  // 元信息行: "Fabric 1.20.1 · Played 5 minutes ago"。
  const loaderLabel = () => {
    const name = fmtLoader(inst().loader);
    return name ? `${name} ${inst().mc_version}` : inst().mc_version;
  };

  const playedLabel = () => {
    const rel = formatRelativeTime(inst().last_played);
    return rel === "never" ? t("instance.neverPlayed") : t("instance.lastPlayed", { rel });
  };

  const onSelectAction = (value: string) => {
    const id = inst().id;
    if (value === "play") props.onPlay?.(id);
    else if (value === "manage") props.onManage?.(id);
    else if (value === "open") props.onOpenDir?.(id);
    else if (value === "export") props.onExport?.(id);
    else if (value === "delete") setConfirmOpen(true);
  };

  return (
    <>
      <div class="flex items-center gap-[14px] glass-card glass-card--hover rounded-card px-[14px] py-[12px] transition-[transform,box-shadow,border-color,background-color] duration-[var(--dur)] ease-app hover:-translate-y-[2px]">
        {/* 行主体:点击进入实例详情。 */}
        <button
          type="button"
          class="flex items-center gap-[14px] flex-1 min-w-0 bg-transparent border-none p-0 text-left cursor-pointer"
          onClick={() => props.onOpen?.(inst().id)}
        >
          {/* 左: 图标 (有 icon 显示图片, 否则渐变 + 首字母)。 */}
          <div class="relative shrink-0 w-[48px] h-[48px] rounded-ctl overflow-hidden flex items-center justify-center bg-gradient-to-br from-a-3 to-a-5 text-white font-bold text-[20px] uppercase select-none">
            <Show when={inst().icon} fallback={<span>{initial()}</span>}>
              <img src={inst().icon} alt="" width="48" height="48" loading="lazy" class="w-full h-full object-cover block" />
            </Show>
            {/* 运行中绿点。 */}
            <Show when={running()}>
              <span
                class="absolute right-[2px] bottom-[2px] w-[11px] h-[11px] rounded-full bg-a-5 shadow-[0_0_0_2px_var(--bg-card)]"
                title={t("instance.running")}
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
        </button>

        {/* 右: Play + ⋮ 菜单(Ark Menu:键盘可达 + 点外部/Esc 关闭)。 */}
        <div class="shrink-0 flex items-center gap-[6px]">
          <PlayButton running={running()} disabled={isLaunching(inst().id)} onClick={() => props.onPlay?.(inst().id)} />
          <Menu.Root positioning={{ placement: "bottom-end" }} onSelect={(d: { value: string }) => onSelectAction(d.value)}>
            <Menu.Trigger
              class="inline-flex items-center justify-center w-[34px] h-[34px] border-none bg-transparent text-dim rounded-ctl cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-glass-hover hover:text-fg data-[state=open]:bg-glass-hover data-[state=open]:text-fg"
              aria-label={t("instance.moreActions")}
            >
              <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
                <circle cx="8" cy="3" r="1.5" />
                <circle cx="8" cy="8" r="1.5" />
                <circle cx="8" cy="13" r="1.5" />
              </svg>
            </Menu.Trigger>
            <Menu.Content>
              <Menu.Item value="play">{running() ? t("instance.stop") : t("instance.play")}</Menu.Item>
              <Menu.Item value="manage">{t("instance.manageInstance")}</Menu.Item>
              <Menu.Item value="open">{t("instance.openGameDir")}</Menu.Item>
              <Menu.Item value="export">{t("instance.exportModpack")}</Menu.Item>
              <Menu.Separator />
              <Menu.Item value="delete" danger>
                {t("instance.deleteInstance")}
              </Menu.Item>
            </Menu.Content>
          </Menu.Root>
        </div>
      </div>

      {/* 删除确认弹窗(Ark Dialog) */}
      <Dialog
        open={confirmOpen()}
        onClose={() => setConfirmOpen(false)}
        label={t("instance.deleteInstance")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-fg break-words">{t("instance.deleteInstanceConfirm", { name: inst().name })}</div>
          <div class="text-[13px] text-dim leading-[1.6]">
            {t("instance.deleteInstanceBodyRow")}
          </div>
          <div class="flex justify-end gap-[10px]">
            <button
              class="h-[34px] px-[16px] border border-glass-border rounded-ctl bg-glass-card text-fg text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-glass-hover"
              onClick={() => setConfirmOpen(false)}
            >
              {t("instance.cancel")}
            </button>
            <button
              class="h-[34px] px-[16px] border-none rounded-ctl bg-danger text-white text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-danger-hover"
              onClick={() => {
                setConfirmOpen(false);
                props.onDelete?.(inst().id);
              }}
            >
              {t("instance.delete")}
            </button>
          </div>
        </div>
      </Dialog>
    </>
  );
}

export default InstanceRow;
