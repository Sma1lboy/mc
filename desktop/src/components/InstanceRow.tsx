import { JSX, Show, createSignal } from "solid-js";
import { PlayButton } from "./PlayButton";
import { InstanceIcon } from "./InstanceIcon";
import { Dialog } from "./Dialog";
import { Menu } from "./Menu";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { Tag } from "./Tag";
import { formatRelativeTime } from "./format";
import { loaderLabel as fmtLoader } from "../util/loaders";
import { isRunning, isLaunching, socialEnabled } from "../store";
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
  /** 领域角色:owner/admin/member;无 = 普通实例(非领域包)。 */
  realmRole?: string;
  /** 是否已装核心。领域薄存根(加入但未「开始同步」)为 false。 */
  installed?: boolean;
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
  /** 多选模式:为真时行左侧出现勾选框,点击行主体改为切换选中(而非进入详情)。 */
  selectable?: boolean;
  /** 当前是否被选中(仅 selectable 时有意义)。 */
  selected?: boolean;
  /** 切换选中回调(传入实例 id)。 */
  onToggleSelect?: (id: string) => void;
}

export function InstanceRow(props: InstanceRowProps): JSX.Element {
  const inst = () => props.instance;
  const [confirmOpen, setConfirmOpen] = createSignal(false);
  // 运行态以进程注册表为准(后端 game://started/exit 实时同步),而非静态的 instance.running。
  const running = () => isRunning(inst().id);

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
      <div class="relative flex items-center gap-[14px] bg-panel shadow-sunken rounded-none px-[14px] py-[12px] transition-[filter] duration-[var(--dur)] ease-app hover:brightness-[1.06]">
        {/* 选中:覆盖一层凸起倒角 + accent 内描边,绕开面板自身的 sunken 阴影,
            不被 hover 滤镜吃掉。 */}
        <Show when={props.selectable && props.selected}>
          <span
            class="pointer-events-none absolute inset-0 rounded-none shadow-raised ring-[2px] ring-inset ring-accent"
            aria-hidden="true"
          />
        </Show>
        {/* 多选模式下的勾选框(纯追加,默认不渲染)。 */}
        <Show when={props.selectable}>
          <button
            type="button"
            role="checkbox"
            aria-checked={!!props.selected}
            aria-label={t("instance.selectRow", { name: inst().name })}
            onClick={() => props.onToggleSelect?.(inst().id)}
            class="relative z-[1] shrink-0 w-[22px] h-[22px] rounded-none border-none flex items-center justify-center cursor-pointer transition-[filter] duration-[var(--dur)] ease-app active:shadow-pressed hover:brightness-110"
            classList={{
              "bg-accent text-accent-text shadow-raised": !!props.selected,
              "bg-panel-2 text-transparent shadow-input": !props.selected,
            }}
          >
            <Icon name="check" size={14} />
          </button>
        </Show>
        {/* 行主体:多选模式下点击切换选中,否则进入实例详情。 */}
        <button
          type="button"
          class="relative z-[1] flex items-center gap-[14px] flex-1 min-w-0 bg-transparent border-none p-0 text-left cursor-pointer"
          onClick={() => (props.selectable ? props.onToggleSelect?.(inst().id) : props.onOpen?.(inst().id))}
        >
          {/* 左: 图标 (有 icon 显示图片, 否则 MC 像素占位)。50px 方块 + 深凹边框。 */}
          <div class="relative shrink-0 w-[50px] h-[50px] rounded-none shadow-input overflow-hidden select-none">
            <InstanceIcon name={inst().name} icon={inst().icon} />
            {/* 运行中熔岩橙指示点。 */}
            <Show when={running()}>
              <span
                class="absolute right-[3px] bottom-[3px] w-[10px] h-[10px] rounded-none bg-accent shadow-raised"
                title={t("instance.running")}
              />
            </Show>
          </div>

          {/* 中: 名称 + 元信息。 */}
          <div class="flex-1 min-w-0 flex flex-col gap-[4px]">
            <div class="flex items-center gap-[6px] min-w-0">
              <span
                class="text-[length:var(--fs-base)] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis"
                title={inst().name}
              >
                {inst().name}
              </span>
              {/* 领域包角标:主办 / 领域 + 未同步(pending)状态。 */}
              <Show when={socialEnabled() && inst().realmRole}>
                <Tag class="shrink-0">
                  {inst().realmRole === "owner" ? t("realm.badgeHost") : t("realm.badgeMember")}
                </Tag>
                <Show when={inst().installed === false}>
                  <Tag class="shrink-0 text-accent">{t("realm.badgePending")}</Tag>
                </Show>
              </Show>
            </div>
            <div class="text-[12px] text-muted whitespace-nowrap overflow-hidden text-ellipsis flex items-center gap-[6px]">
              <span>{loaderLabel()}</span>
              <span class="opacity-50">·</span>
              <span>{playedLabel()}</span>
            </div>
          </div>
        </button>

        {/* 右: Play + ⋮ 菜单(Ark Menu:键盘可达 + 点外部/Esc 关闭)。 */}
        <div class="relative z-[1] shrink-0 flex items-center gap-[8px]">
          <PlayButton
            running={running()}
            disabled={isLaunching(inst().id) || inst().installed === false}
            onClick={() => props.onPlay?.(inst().id)}
          />
          <Menu.Root positioning={{ placement: "bottom-end" }} onSelect={(d: { value: string }) => onSelectAction(d.value)}>
            <Menu.Trigger
              class="inline-flex items-center justify-center w-[36px] h-[36px] border-none rounded-none bg-panel-3 text-sub shadow-raised cursor-pointer transition-[filter,color] duration-[var(--dur)] ease-app hover:brightness-110 hover:text-fg active:shadow-pressed data-[state=open]:shadow-pressed data-[state=open]:text-fg"
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
        contentClass="w-[360px] max-w-[calc(100vw-48px)]"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-strong break-words">{t("instance.deleteInstanceConfirm", { name: inst().name })}</div>
          <div class="text-[13px] text-muted leading-[1.6]">
            {t("instance.deleteInstanceBodyRow")}
          </div>
          <div class="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmOpen(false)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                setConfirmOpen(false);
                props.onDelete?.(inst().id);
              }}
            >
              {t("instance.delete")}
            </Button>
          </div>
        </div>
      </Dialog>
    </>
  );
}

export default InstanceRow;
