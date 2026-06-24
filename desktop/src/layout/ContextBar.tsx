import {
  Component,
  For,
  Show,
  createResource,
  createSignal,
} from "solid-js";
import { AccountDialog, Avatar, EmptyState, ErrorState, Heading, Menu, Panel, Spinner } from "../components";
import { skinBodyUrl } from "../components/Avatar";
import { ACCENT_BTN } from "../components/styles";
import { api } from "../ipc/api";
import { accountKindLabel } from "../util/accounts";
import { t } from "../i18n";
import type { AccountSummary } from "../ipc/types";
import "./ContextBar.css"; // 残留:@keyframes ctx-pulse(骨架脉冲)

/**
 * ContextBar —— 340px 右侧上下文栏。新 IA 下已从外壳移除(showContext 全 false),
 * 组件暂留备用:某页日后需要右栏时把 route 的 showContext 置 true 即可恢复。
 *
 * 三块:
 *   1. Playing as —— 账号选择器(头像 + 用户名 + 下拉箭头),展开切换账号。
 *   2. Friends    —— 好友列表占位(社交功能未接入时给空态)。
 *   3. News       —— 新闻/动态占位。
 *
 * 数据:createResource(() => api.listAccounts())。loading / 空 / 错误三态都处理。
 * Blocky:石质底(stone)+ 凹陷倒角,左侧分隔(border-titlebar)。
 */

// 元信息列(用户名 + 类型),可截断。
const META = "flex flex-col gap-px min-w-0 flex-[1_1_auto]";
// 用户名(单行截断)。
const NAME =
  "text-[var(--fs-base)] font-medium text-fg whitespace-nowrap overflow-hidden text-ellipsis";
// 账号类型小字。
const KIND = "text-[11px] text-muted";

const ChevronDown = () => (
  <svg class="w-[16px] h-[16px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m6 9 6 6 6-6" />
  </svg>
);

const ContextBar: Component = () => {
  // 账号列表。refetch 用于切换账号后刷新 selected 标记。
  const [accounts, { refetch }] = createResource<AccountSummary[]>(() => api.listAccounts());

  // 新闻/动态:来自 mc-server(未运行/不可达则空,降级到占位)。
  const [news] = createResource(() => api.fetchNews());

  // 切换账号时的错误提示(后端命令缺失/失败时显示,不崩 UI)
  const [switchErr, setSwitchErr] = createSignal<string | null>(null);
  // 登录弹窗(工作台布局的账号入口 —— 之前这里只有「前往设置登录」却无处可登录)。
  const [loginOpen, setLoginOpen] = createSignal(false);

  const onLoggedIn = () => {
    setLoginOpen(false);
    void refetch();
  };

  // 当前账号:优先 selected,否则第一个。
  const current = (): AccountSummary | undefined => {
    const list = accounts();
    if (!list || list.length === 0) return undefined;
    return list.find((a) => a.selected) ?? list[0];
  };


  // 切换到指定账号:调后端 select_account(对应 core 的 AccountStore::select),
  // 成功后收起下拉并刷新列表。命令不存在/失败时记录错误、不阻塞 UI。
  const pick = async (acc: AccountSummary) => {
    setSwitchErr(null);
    if (acc.selected) return;
    try {
      await api.selectAccount(acc.uuid);
      await refetch();
    } catch (e) {
      setSwitchErr(typeof e === "string" ? e : t("account.switchFailed"));
    }
  };

  return (
    <aside
      class="[grid-row:1] [grid-column:2] w-[340px] h-full box-border stone shadow-sunken border-l border-titlebar p-[16px] flex flex-col gap-[20px] overflow-y-auto"
      aria-label={t("account.contextAria")}
    >
      {/* ===== Playing as ===== */}
      <section class="flex flex-col gap-[8px]">
        <Heading size="mini" as="h3" class="text-sub">{t("account.sectionCurrent")}</Heading>

        <Show
          when={!accounts.loading}
          fallback={
            <div
              class="account-card-skeleton h-[56px] bg-panel-2 shadow-sunken"
              aria-busy="true"
            />
          }
        >
          {/* 错误态:list_accounts 失败 */}
          <Show
            when={!accounts.error}
            fallback={<ErrorState compact message={t("account.contextLoadFailed")} onRetry={() => void refetch()} />}
          >
            {/* 空态:无任何账号 */}
            <Show
              when={(accounts()?.length ?? 0) > 0}
              fallback={
                <Panel variant="sunken" class="flex flex-col gap-[10px] p-[14px]">
                  <div class="flex flex-col gap-[2px]">
                    <span class="text-[var(--fs-base)] text-fg">{t("account.noAccount")}</span>
                    <span class="text-[12px] text-muted">{t("account.noAccountHint")}</span>
                  </div>
                  <button class={`${ACCENT_BTN} motion-reduce:transition-none`} onClick={() => setLoginOpen(true)}>
                    {t("account.loginOrAdd")}
                  </button>
                </Panel>
              }
            >
              {/* 当前账号的全身皮肤预览(像素硬边);mc-heads 取不到时 onError 隐藏,不留空洞。 */}
              <Show when={current()}>
                <div class="flex justify-center pt-[2px] pb-[12px]">
                  <img
                    src={skinBodyUrl(current()!.uuid)}
                    alt=""
                    class="h-[150px] w-auto object-contain [image-rendering:pixelated] drop-shadow-[0_4px_12px_rgba(0,0,0,0.28)]"
                    onError={(e) => (e.currentTarget.style.display = "none")}
                  />
                </div>
              </Show>

              {/* 账号切换:Ark Menu(键盘可达 + 点外部/Esc 自动收起) */}
              <Menu.Root
                positioning={{ placement: "bottom", sameWidth: true }}
                onSelect={(d: { value: string }) => {
                  if (d.value === "__add__") {
                    setLoginOpen(true);
                    return;
                  }
                  const acc = accounts()?.find((a) => a.uuid === d.value);
                  if (acc) void pick(acc);
                }}
              >
                <Menu.Trigger
                  class="group flex items-center gap-[10px] w-full p-[10px] bg-panel shadow-sunken cursor-pointer text-left transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-panel-2 data-[state=open]:shadow-input motion-reduce:transition-none"
                  aria-label={t("account.switchAccount")}
                >
                  <span class="w-[36px] h-[36px] flex-shrink-0 shadow-raised grid place-items-center text-[15px] font-semibold text-accent-text bg-accent">
                    <Avatar kind={current()?.kind} uuid={current()?.uuid} />
                  </span>
                  <span class={META}>
                    <span class={NAME}>{current()?.username}</span>
                    <span class={KIND}>{accountKindLabel(current()?.kind)}</span>
                  </span>
                  <span
                    class="flex-shrink-0 text-muted grid place-items-center transition-transform duration-[var(--dur)] ease-app group-data-[state=open]:rotate-180 motion-reduce:transition-none"
                    aria-hidden="true"
                  >
                    <ChevronDown />
                  </span>
                </Menu.Trigger>

                <Menu.Content>
                  <For each={accounts()}>
                    {(acc) => (
                      <Menu.ItemRaw
                        value={acc.uuid}
                        class="flex items-center gap-[10px] p-[8px] rounded-none cursor-pointer select-none transition-[background] duration-[var(--dur)] ease-app data-[highlighted]:bg-panel-2 motion-reduce:transition-none"
                        classList={{
                          "bg-panel-3": acc.selected,
                        }}
                      >
                        <span class="w-[30px] h-[30px] flex-shrink-0 shadow-raised grid place-items-center text-[13px] font-semibold text-accent-text bg-accent">
                          <Avatar kind={acc.kind} uuid={acc.uuid} />
                        </span>
                        <span class={META}>
                          <span class={NAME}>{acc.username}</span>
                          <span class={KIND}>{accountKindLabel(acc.kind)}</span>
                        </span>
                        <Show when={acc.selected}>
                          <span class="text-accent text-[14px] flex-shrink-0" aria-hidden="true">✓</span>
                        </Show>
                      </Menu.ItemRaw>
                    )}
                  </For>
                  <Menu.ItemRaw
                    value="__add__"
                    class="flex items-center gap-[10px] p-[8px] mt-[2px] rounded-none cursor-pointer select-none border-t border-titlebar text-accent transition-[background] duration-[var(--dur)] ease-app data-[highlighted]:bg-panel-2 motion-reduce:transition-none"
                  >
                    <span class="w-[30px] h-[30px] flex-shrink-0 shadow-raised grid place-items-center text-[18px] font-semibold bg-panel-3" aria-hidden="true">
                      +
                    </span>
                    <span class="text-[13px] font-medium">{t("account.loginOrAdd")}</span>
                  </Menu.ItemRaw>
                </Menu.Content>
              </Menu.Root>

              {/* 切换错误提示 */}
              <Show when={switchErr()}>
                <div class="mt-[6px] text-[12px] text-danger-text">{switchErr()}</div>
              </Show>
            </Show>
          </Show>
        </Show>
      </section>

      {/* ===== Friends ===== */}
      <section class="flex flex-col gap-[8px]">
        <Heading size="mini" as="h3" class="text-sub">{t("account.sectionFriends")}</Heading>
        {/* 社交功能未接入:空态占位。接入后此处渲染好友 + 在线状态点。 */}
        <EmptyState compact title={t("account.friendsEmpty")} hint={t("account.friendsHint")} />
      </section>

      {/* ===== News ===== */}
      <section class="flex flex-col gap-[8px]">
        <Heading size="mini" as="h3" class="text-sub">{t("account.sectionNews")}</Heading>
        <Show
          when={!news.loading}
          fallback={<div class="flex justify-center py-[14px]"><Spinner size={16} /></div>}
        >
          <Show
            when={!news.error && (news()?.length ?? 0) > 0}
            fallback={<EmptyState compact title={t("account.newsEmpty")} hint={t("account.newsHint")} />}
          >
            <div class="flex flex-col gap-[8px]">
              <For each={news()!.slice(0, 5)}>
                {(item) => {
                  const inner = (
                    <>
                      <div class="flex items-baseline justify-between gap-[8px]">
                        <span class="text-[13px] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                          {item.title}
                        </span>
                        <span class="text-[11px] text-muted shrink-0 [font-variant-numeric:tabular-nums]">{item.date}</span>
                      </div>
                      <div class="text-[12px] text-sub leading-[1.5] line-clamp-3">{item.body}</div>
                    </>
                  );
                  const cls =
                    "flex flex-col gap-[3px] p-[10px] bg-panel shadow-sunken transition-[box-shadow] duration-[var(--dur)] ease-app";
                  return (
                    <Show
                      when={item.url}
                      fallback={<div class={cls}>{inner}</div>}
                    >
                      <a href={item.url!} class={`${cls} no-underline cursor-pointer hover:shadow-raised`}>
                        {inner}
                      </a>
                    </Show>
                  );
                }}
              </For>
            </div>
          </Show>
        </Show>
      </section>

      <Show when={loginOpen()}>
        <AccountDialog onClose={() => setLoginOpen(false)} onDone={onLoggedIn} />
      </Show>
    </aside>
  );
};

export default ContextBar;
