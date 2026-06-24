import { Component, For, Show, createResource, createSignal } from "solid-js";
import { Menu } from "./Menu";
import { Avatar } from "./Avatar";
import { AccountDialog } from "./AccountDialog";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { accountKindLabel } from "../util/accounts";
import { t } from "../i18n";
import type { AccountSummary } from "../ipc/types";

/* ============================================================================
 * AccountMenu —— 持久的账号入口 + 切换器(替代旧 ContextBar 的「Playing as」)。
 *
 * 新 IA 下右栏退役,账号收成 rail 头像方块 / 顶栏芯片。本组件把「显示当前账号 +
 * 下拉切换 + 添加账号(AccountDialog 登录)」收敛成一个可复用件,两处触发形态:
 *   variant="avatar" —— rail 底部 36px 头像方块(向右展开)。
 *   variant="chip"   —— 顶栏账号芯片(头像 + 名 + 在线点,向下展开)。
 * 自带 listAccounts / selectAccount / 登录弹窗,调用方零状态。
 * ========================================================================== */

const META = "flex flex-col gap-px min-w-0 flex-[1_1_auto] text-left";
const NAME =
  "text-[13px] font-medium text-strong leading-tight whitespace-nowrap overflow-hidden text-ellipsis max-w-[160px]";

export interface AccountMenuProps {
  /** 触发器形态,默认 avatar。 */
  variant?: "avatar" | "chip";
}

export const AccountMenu: Component<AccountMenuProps> = (props) => {
  const [accounts, { refetch }] = createResource<AccountSummary[]>(() => api.listAccounts());
  const [loginOpen, setLoginOpen] = createSignal(false);

  // 当前账号:优先 selected,否则第一个。
  const current = (): AccountSummary | undefined => {
    const list = accounts();
    if (!list || list.length === 0) return undefined;
    return list.find((a) => a.selected) ?? list[0];
  };
  const online = (): boolean => !!current() && current()!.kind !== "offline";

  // 切到指定账号(已选则忽略);失败 toast 提示,不崩 UI。
  const pick = async (acc: AccountSummary): Promise<void> => {
    if (acc.selected) return;
    try {
      await api.selectAccount(acc.uuid);
      await refetch();
    } catch (e) {
      toast({ type: "error", message: typeof e === "string" ? e : t("account.switchFailed") });
    }
  };

  const onSelect = (d: { value: string }): void => {
    if (d.value === "__add__") {
      setLoginOpen(true);
      return;
    }
    const acc = accounts()?.find((a) => a.uuid === d.value);
    if (acc) void pick(acc);
  };

  const isChip = (): boolean => props.variant === "chip";

  return (
    <>
      <Menu.Root
        positioning={{ placement: isChip() ? "bottom-end" : "right-start", gutter: 6 }}
        onSelect={onSelect}
      >
        <Menu.Trigger
          aria-label={t("account.switchAccount")}
          class={
            isChip()
              ? "flex items-center gap-[10px] shrink-0 py-[7px] pl-[8px] pr-[12px] bg-panel-3 shadow-raised rounded-none cursor-pointer transition-[box-shadow,filter] duration-[var(--dur)] ease-app hover:brightness-110 active:shadow-pressed data-[state=open]:shadow-pressed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent motion-reduce:transition-none"
              : "block w-[36px] h-[36px] p-0 border-none bg-transparent shadow-raised rounded-none overflow-hidden cursor-pointer transition-transform duration-[var(--dur)] ease-app active:scale-[0.94] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent motion-reduce:transition-none"
          }
        >
          <Show
            when={isChip()}
            fallback={<Avatar kind={current()?.kind} uuid={current()?.uuid} />}
          >
            <span class="w-[28px] h-[28px] shrink-0 overflow-hidden rounded-[2px]">
              <Avatar kind={current()?.kind} uuid={current()?.uuid} />
            </span>
            <span class={META}>
              <span class={NAME}>{current()?.username ?? t("account.loginOrAdd")}</span>
              <Show when={current()}>
                <span class="flex items-center gap-[5px] text-[11px] text-muted leading-tight">
                  <span
                    class={`w-[6px] h-[6px] rounded-none shrink-0 ${online() ? "bg-[#5fb84e]" : "bg-faint"}`}
                    aria-hidden="true"
                  />
                  {accountKindLabel(current()!.kind)}
                </span>
              </Show>
            </span>
          </Show>
        </Menu.Trigger>

        <Menu.Content class="min-w-[224px]">
          <For each={accounts()}>
            {(acc) => (
              <Menu.ItemRaw
                value={acc.uuid}
                class="flex items-center gap-[10px] p-[8px] rounded-none cursor-pointer select-none transition-[background] duration-[var(--dur)] ease-app data-[highlighted]:bg-panel-3 motion-reduce:transition-none"
              >
                <span class="w-[30px] h-[30px] flex-shrink-0 shadow-raised overflow-hidden grid place-items-center bg-accent">
                  <Avatar kind={acc.kind} uuid={acc.uuid} />
                </span>
                <span class={META}>
                  <span class={NAME}>{acc.username}</span>
                  <span class="text-[11px] text-muted">{accountKindLabel(acc.kind)}</span>
                </span>
                <Show when={acc.selected}>
                  <span class="text-accent text-[14px] flex-shrink-0" aria-hidden="true">
                    ✓
                  </span>
                </Show>
              </Menu.ItemRaw>
            )}
          </For>
          <Menu.ItemRaw
            value="__add__"
            class="flex items-center gap-[10px] p-[8px] mt-[2px] rounded-none cursor-pointer select-none border-t border-titlebar text-accent transition-[background] duration-[var(--dur)] ease-app data-[highlighted]:bg-panel-3 motion-reduce:transition-none"
          >
            <span
              class="w-[30px] h-[30px] flex-shrink-0 shadow-raised grid place-items-center text-[18px] font-semibold bg-panel-3"
              aria-hidden="true"
            >
              +
            </span>
            <span class="text-[13px] font-medium">{t("account.loginOrAdd")}</span>
          </Menu.ItemRaw>
        </Menu.Content>
      </Menu.Root>

      <Show when={loginOpen()}>
        <AccountDialog
          onClose={() => setLoginOpen(false)}
          onDone={() => {
            setLoginOpen(false);
            void refetch();
          }}
        />
      </Show>
    </>
  );
};

export default AccountMenu;
