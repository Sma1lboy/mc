import { Component, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Icon } from "./Icon";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { friendRequests, refreshFriendRequests, refreshFriends } from "../store";
import { t } from "../i18n";
import type { UserBrief } from "../ipc/bindings";

const label = (u: UserBrief) => u.username || u.id.slice(0, 8);

/**
 * NotificationCenter —— 顶栏右上角的通知中心(铃铛 + 未读角标)。
 * 未读数 = 收到的好友请求数;点开保持挂载的下拉,内联接受/拒绝每条请求。
 * 数据来自 store 的 friendRequests(单一真相 + 连续 30s 轮询),不在此另起轮询。
 *
 * 扩展点:这里目前只渲染「好友请求」一类通知。后续接入领域邀请(realm invites)等时,
 * 可把每一类通知抽成一个分组 section(标题 + For 列表 + 各自的接受/拒绝处理),
 * 角标合计各类未读数即可 —— 结构已按「分组」预留。
 */
export const NotificationCenter: Component = () => {
  const [open, setOpen] = createSignal(false);
  const requests = friendRequests;
  const unread = () => (requests() ?? []).length;
  const [busy, setBusy] = createSignal(false);
  let rootEl: HTMLDivElement | undefined;

  onMount(() => {
    const onDoc = (e: MouseEvent) => {
      if (rootEl && !rootEl.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    onCleanup(() => document.removeEventListener("mousedown", onDoc));
  });

  const fail = (e: unknown) => toast({ type: "error", message: t("friend.opError", { err: String(e) }) });

  async function act(fn: () => Promise<void>) {
    if (busy()) return;
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  const accept = (u: UserBrief) =>
    act(async () => {
      await api.friendAccept(u.id);
      void refreshFriendRequests();
      void refreshFriends();
    });
  const decline = (u: UserBrief) =>
    act(async () => {
      await api.friendDecline(u.id);
      void refreshFriendRequests();
    });

  return (
    <div ref={rootEl} class="relative">
      <button
        type="button"
        class="relative inline-flex items-center justify-center h-[26px] w-[30px] bg-panel-2 shadow-sunken text-fg cursor-pointer hover:brightness-110 transition-[filter] duration-150"
        onClick={() => setOpen((o) => !o)}
        title={t("notification.title")}
      >
        <span class="grid place-items-center text-accent">
          <Icon name="bell" size={14} />
        </span>
        {/* 未读角标:好友请求数;为 0 时隐藏。 */}
        <Show when={unread() > 0}>
          <span class="absolute top-[1px] right-[1px] min-w-[14px] h-[14px] px-[3px] grid place-items-center bg-danger text-danger-text text-[10px] leading-none tabular-nums">
            {unread()}
          </span>
        </Show>
      </button>

      {/* 下拉体保持挂载、用 hidden 切换显隐(同账号 chip 的 keep-alive 模式)。 */}
      <div
        class="absolute right-0 top-[calc(100%+6px)] w-[300px] bg-panel border border-titlebar shadow-raised rounded-none z-[200] p-[16px]"
        classList={{ hidden: !open() }}
      >
        <div class="text-[13px] text-strong font-display mb-[8px]">{t("notification.title")}</div>

        <Show
          when={unread() > 0}
          fallback={<p class="text-[12px] text-faint">{t("notification.empty")}</p>}
        >
          {/* 好友请求分组(后续可在下方追加领域邀请等分组)。 */}
          <div class="flex flex-col gap-[4px]">
            <div class="text-[11px] text-muted">{t("friend.requests")}</div>
            <For each={requests()}>
              {(u) => (
                <div class="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
                  <span class="text-[13px] text-fg truncate flex-1">{label(u)}</span>
                  <button
                    class="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                    disabled={busy()}
                    onClick={() => void accept(u)}
                  >
                    {t("friend.accept")}
                  </button>
                  <button
                    class="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                    disabled={busy()}
                    onClick={() => void decline(u)}
                  >
                    {t("friend.decline")}
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
};
