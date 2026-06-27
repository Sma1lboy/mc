import { Component, createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { Icon } from "./Icon";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import {
  notifications,
  markNotificationsRead,
  refreshNotifications,
  refreshFriendRequests,
  refreshFriends,
} from "../store";
import { t } from "../i18n";
import type { Notification } from "../ipc/bindings";

/** 通知里 actor 的展示名:用户名优先,缺失回退 id 前缀。 */
const actorName = (n: Notification) => n.actor_username || (n.actor_id ? n.actor_id.slice(0, 8) : "");

/**
 * NotificationCenter —— 顶栏右上角的通知中心(铃铛 + 未读角标)。
 * 数据来自 store 的 notifications()(服务端 typed 通知,单一真相 + 连续 30s 轮询),
 * 不在此另起轮询。未读数 = 未读通知数;点开下拉时一次性标记全部已读(清角标)。
 *
 * 按 kind 渲染每条通知:
 *  - friend_request   可操作(接受/拒绝)
 *  - friend_accepted  信息性
 *  - realm_invite     信息性
 *  - 其他未知 kind     兜底单行,不崩
 */
export const NotificationCenter: Component = () => {
  const [open, setOpen] = createSignal(false);
  const items = notifications;
  const unread = () => (items() ?? []).filter((n) => !n.read).length;
  const [busy, setBusy] = createSignal(false);
  let rootEl: HTMLDivElement | undefined;

  onMount(() => {
    const onDoc = (e: MouseEvent) => {
      if (rootEl && !rootEl.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    onCleanup(() => document.removeEventListener("mousedown", onDoc));
  });

  // 切换下拉:打开时标记全部已读(清角标)。
  function toggle() {
    setOpen((o) => {
      const next = !o;
      if (next) void markNotificationsRead();
      return next;
    });
  }

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

  const accept = (n: Notification) =>
    act(async () => {
      if (!n.actor_id) return;
      await api.friendAccept(n.actor_id);
      void refreshFriends();
      void refreshFriendRequests();
      void refreshNotifications();
    });
  const decline = (n: Notification) =>
    act(async () => {
      if (!n.actor_id) return;
      await api.friendDecline(n.actor_id);
      void refreshFriendRequests();
      void refreshNotifications();
    });

  return (
    <div ref={rootEl} class="relative">
      <button
        type="button"
        class="relative inline-flex items-center justify-center h-[26px] w-[30px] bg-panel-2 shadow-sunken text-fg cursor-pointer hover:brightness-110 transition-[filter] duration-150"
        onClick={toggle}
        title={t("notification.title")}
      >
        <span class="grid place-items-center text-accent">
          <Icon name="bell" size={14} />
        </span>
        {/* 未读角标:未读通知数;为 0 时隐藏。 */}
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
          when={(items() ?? []).length > 0}
          fallback={<p class="text-[12px] text-faint">{t("notification.empty")}</p>}
        >
          <div class="flex flex-col gap-[4px]">
            <For each={items()}>
              {(n) => (
                <div class="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
                  <Switch
                    fallback={
                      <span class="text-[13px] text-fg truncate flex-1">{n.kind}</span>
                    }
                  >
                    <Match when={n.kind === "friend_request"}>
                      <span class="text-[13px] text-fg truncate flex-1">
                        {t("notification.friendRequest", { name: actorName(n) })}
                      </span>
                      <Show when={n.actor_id}>
                        <button
                          class="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy()}
                          onClick={() => void accept(n)}
                        >
                          {t("friend.accept")}
                        </button>
                        <button
                          class="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy()}
                          onClick={() => void decline(n)}
                        >
                          {t("friend.decline")}
                        </button>
                      </Show>
                    </Match>
                    <Match when={n.kind === "friend_accepted"}>
                      <span class="text-[13px] text-fg truncate flex-1">
                        {t("notification.friendAccepted", { name: actorName(n) })}
                      </span>
                    </Match>
                    <Match when={n.kind === "realm_invite"}>
                      <span class="text-[13px] text-fg truncate flex-1">
                        {t("notification.realmInvite", {
                          name: actorName(n),
                          realm: n.realm_name || n.realm_id || "",
                        })}
                      </span>
                    </Match>
                  </Switch>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
};
