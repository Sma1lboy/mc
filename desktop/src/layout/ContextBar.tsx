import {
  Component,
  For,
  Show,
  createResource,
  createSignal,
} from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { AccountDialog, Avatar, EmptyState, Menu } from "../components";
import type { AccountSummary, AccountKind } from "../ipc/types";
import "./ContextBar.css"; // 残留:@keyframes ctx-pulse(骨架脉冲)

/**
 * ContextBar —— 340px 右侧上下文栏(Home 页内容)。
 *
 * 三块:
 *   1. Playing as —— 账号选择器(头像 + 用户名 + 下拉箭头),展开切换账号。
 *   2. Friends    —— 好友列表占位(社交功能未接入时给空态)。
 *   3. News       —— 新闻/动态占位。
 *
 * 数据:createResource(() => invoke('list_accounts'))。loading / 空 / 错误三态都处理。
 * 背景 --n-2,左侧分隔(border-left)。
 */

// 账号类型 → 中文标签。AccountKind 在后端 serde 为小写(offline/microsoft/yggdrasil)。
const KIND_LABEL: Record<AccountKind, string> = {
  offline: "离线",
  microsoft: "微软",
  yggdrasil: "外置登录",
};

// 通用栏目标题(灰色小标题)。
const HEADING =
  "m-0 text-[13px] font-semibold text-dim tracking-[0.2px]";
// 元信息列(用户名 + 类型),可截断。
const META = "flex flex-col gap-px min-w-0 flex-[1_1_auto]";
// 用户名(单行截断)。
const NAME =
  "text-[var(--fs-base)] font-medium text-fg whitespace-nowrap overflow-hidden text-ellipsis";
// 账号类型小字。
const KIND = "text-[11px] text-dim";

const ChevronDown = () => (
  <svg class="w-[16px] h-[16px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
       stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <path d="m6 9 6 6 6-6" />
  </svg>
);

const ContextBar: Component = () => {
  // 账号列表。refetch 用于切换账号后刷新 selected 标记。
  const [accounts, { refetch }] = createResource<AccountSummary[]>(async () => {
    return await invoke<AccountSummary[]>("list_accounts");
  });

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
      await invoke("select_account", { uuid: acc.uuid });
      await refetch();
    } catch (e) {
      setSwitchErr(typeof e === "string" ? e : "切换账号失败");
    }
  };

  return (
    <aside
      class="[grid-row:1] [grid-column:2] w-[340px] h-full box-border glass-panel border-l border-glass-divider p-[16px] flex flex-col gap-[20px] overflow-y-auto"
      aria-label="上下文信息"
    >
      {/* ===== Playing as ===== */}
      <section class="flex flex-col gap-[8px]">
        <h3 class={HEADING}>当前账号</h3>

        <Show
          when={!accounts.loading}
          fallback={
            <div
              class="account-card-skeleton h-[56px] rounded-ctl bg-n-5"
              aria-busy="true"
            />
          }
        >
          {/* 错误态:list_accounts 失败 */}
          <Show
            when={!accounts.error}
            fallback={
              <div class="flex items-center justify-between gap-[8px] p-[12px] border border-glass-border rounded-ctl bg-glass-card text-[13px] text-dim">
                <span>账号载入失败</span>
                <button
                  class="border border-glass-border bg-glass-card text-fg rounded-xs px-[10px] py-[4px] text-[12px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-glass-hover motion-reduce:transition-none"
                  onClick={() => refetch()}
                >
                  重试
                </button>
              </div>
            }
          >
            {/* 空态:无任何账号 */}
            <Show
              when={(accounts()?.length ?? 0) > 0}
              fallback={
                <div class="flex flex-col gap-[10px] p-[14px] border border-dashed border-glass-border rounded-ctl bg-glass-card">
                  <div class="flex flex-col gap-[2px]">
                    <span class="text-[var(--fs-base)] text-fg">尚未添加账号</span>
                    <span class="text-[12px] text-dim">登录微软正版,或添加一个离线账号</span>
                  </div>
                  <button
                    class="h-[34px] rounded-ctl border-none bg-a-5 text-white text-[13px] font-semibold cursor-pointer transition-opacity duration-[var(--dur)] ease-app hover:opacity-90 motion-reduce:transition-none"
                    onClick={() => setLoginOpen(true)}
                  >
                    登录 / 添加账号
                  </button>
                </div>
              }
            >
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
                  class="group flex items-center gap-[10px] w-full p-[10px] border border-glass-border rounded-ctl bg-glass-card cursor-pointer text-left transition-[background-color,border-color] duration-[var(--dur)] ease-app hover:bg-glass-hover hover:border-a-4 data-[state=open]:border-a-4 motion-reduce:transition-none"
                  aria-label="切换账号"
                >
                  <span class="w-[36px] h-[36px] flex-shrink-0 rounded-xs grid place-items-center text-[15px] font-semibold text-white bg-[linear-gradient(135deg,var(--a-3),var(--a-5))]">
                    <Avatar kind={current()?.kind} uuid={current()?.uuid} />
                  </span>
                  <span class={META}>
                    <span class={NAME}>{current()?.username}</span>
                    <span class={KIND}>{current() ? KIND_LABEL[current()!.kind] : ""}</span>
                  </span>
                  <span
                    class="flex-shrink-0 text-dim grid place-items-center transition-transform duration-[var(--dur)] ease-app group-data-[state=open]:rotate-180 motion-reduce:transition-none"
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
                        class="flex items-center gap-[10px] p-[8px] rounded-xs cursor-pointer select-none transition-[background] duration-[var(--dur)] ease-app data-[highlighted]:bg-glass-hover motion-reduce:transition-none"
                        classList={{
                          "bg-[color-mix(in_srgb,var(--a-4)_14%,transparent)]": acc.selected,
                        }}
                      >
                        <span class="w-[30px] h-[30px] flex-shrink-0 rounded-xs grid place-items-center text-[13px] font-semibold text-white bg-[linear-gradient(135deg,var(--a-3),var(--a-5))]">
                          <Avatar kind={acc.kind} uuid={acc.uuid} />
                        </span>
                        <span class={META}>
                          <span class={NAME}>{acc.username}</span>
                          <span class={KIND}>{KIND_LABEL[acc.kind]}</span>
                        </span>
                        <Show when={acc.selected}>
                          <span class="text-a-5 text-[14px] flex-shrink-0" aria-hidden="true">✓</span>
                        </Show>
                      </Menu.ItemRaw>
                    )}
                  </For>
                  <Menu.ItemRaw
                    value="__add__"
                    class="flex items-center gap-[10px] p-[8px] mt-[2px] rounded-xs cursor-pointer select-none border-t border-glass-border text-a-6 transition-[background] duration-[var(--dur)] ease-app data-[highlighted]:bg-glass-hover motion-reduce:transition-none"
                  >
                    <span class="w-[30px] h-[30px] flex-shrink-0 rounded-xs grid place-items-center text-[18px] font-semibold bg-glass-card" aria-hidden="true">
                      +
                    </span>
                    <span class="text-[13px] font-medium">登录 / 添加账号</span>
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
        <h3 class={HEADING}>好友</h3>
        {/* 社交功能未接入:空态占位。接入后此处渲染好友 + 在线状态点。 */}
        <EmptyState compact title="暂无好友" hint="联机/社交功能开发中" />
      </section>

      {/* ===== News ===== */}
      <section class="flex flex-col gap-[8px]">
        <h3 class={HEADING}>动态</h3>
        {/* 新闻 feed 未接入:空态占位。接入后渲染公告/更新卡片列表。 */}
        <EmptyState compact title="暂无动态" hint="敬请期待" />
      </section>

      <Show when={loginOpen()}>
        <AccountDialog onClose={() => setLoginOpen(false)} onDone={onLoggedIn} />
      </Show>
    </aside>
  );
};

export default ContextBar;
