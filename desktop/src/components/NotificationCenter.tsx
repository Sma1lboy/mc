import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { Icon } from "./Icon";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import {
  useAppStore,
  markNotificationsRead,
  refreshNotifications,
  refreshFriendRequests,
  refreshFriends,
  openInstance,
} from "../store";
import { t, useLang } from "../i18n";
import type { Notification } from "../ipc/bindings";

/** 通知里 actor 的展示名:用户名优先,缺失回退 id 前缀。 */
const actorName = (n: Notification) => n.actor_username || (n.actor_id ? n.actor_id.slice(0, 8) : "");

/**
 * NotificationCenter —— 顶栏右上角的通知中心(铃铛 + 未读角标)。
 * 数据来自 store 的 notifications(服务端 typed 通知,单一真相 + 连续 30s 轮询),
 * 不在此另起轮询。未读数 = 未读通知数;点开下拉时一次性标记全部已读(清角标)。
 *
 * 按 kind 渲染每条通知:
 *  - friend_request   可操作(接受/拒绝)
 *  - friend_accepted  信息性
 *  - realm_invite     信息性
 *  - 其他未知 kind     兜底单行,不崩
 */
export function NotificationCenter(): React.ReactElement {
  useLang();
  const [open, setOpen] = useState(false);
  const items = useAppStore((s) => s.notifications);
  const instances = useAppStore((s) => s.instances);
  const unread = (items ?? []).filter((n) => !n.read).length;
  const [busy, setBusy] = useState(false);
  const rootEl = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (rootEl.current && !rootEl.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);

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
    if (busy) return;
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

  function renderNotification(n: Notification): React.ReactNode {
    if (n.kind === "friend_request") {
      return (
        <>
          <span className="text-[13px] text-fg truncate flex-1">
            {t("notification.friendRequest", { name: actorName(n) })}
          </span>
          {n.actor_id && (
            <>
              <button
                className="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                disabled={busy}
                onClick={() => void accept(n)}
              >
                {t("friend.accept")}
              </button>
              <button
                className="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                disabled={busy}
                onClick={() => void decline(n)}
              >
                {t("friend.decline")}
              </button>
            </>
          )}
        </>
      );
    }
    if (n.kind === "friend_accepted") {
      return (
        <span className="text-[13px] text-fg truncate flex-1">
          {t("notification.friendAccepted", { name: actorName(n) })}
        </span>
      );
    }
    if (n.kind === "realm_invite") {
      // 「前往」:仅当本地有实例绑定到该领域时出现 → 进实例(落到「领域」tab)并关闭下拉。
      // 被邀请但本地尚未加入(无绑定实例)时不渲染按钮,仅作信息提示。
      const bound = n.realm_id ? (instances ?? []).find((i) => i.realm?.realm_id === n.realm_id) : undefined;
      return (
        <>
          <span className="text-[13px] text-fg truncate flex-1">
            {t("notification.realmInvite", {
              name: actorName(n),
              realm: n.realm_name || n.realm_id || "",
            })}
          </span>
          {bound && (
            <button
              className="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer"
              onClick={() => {
                openInstance(bound.id);
                setOpen(false);
              }}
            >
              {t("notification.realmInviteGo")}
            </button>
          )}
        </>
      );
    }
    return <span className="text-[13px] text-fg truncate flex-1">{n.kind}</span>;
  }

  return (
    <div ref={rootEl} className="relative">
      <button
        type="button"
        className="relative inline-flex items-center justify-center w-[30px] h-[30px] rounded-none border-none bg-transparent text-dim cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-panel-2 hover:text-fg data-[state=open]:bg-panel-2 data-[state=open]:text-fg [-webkit-app-region:no-drag]"
        data-state={open ? "open" : "closed"}
        onClick={toggle}
        title={t("notification.title")}
      >
        <span className="grid place-items-center">
          <Icon name="bell" size={16} />
        </span>
        {/* 未读角标:未读通知数;为 0 时隐藏。 */}
        {unread > 0 && (
          <span className="absolute top-[1px] right-[1px] min-w-[14px] h-[14px] px-[3px] grid place-items-center bg-danger text-danger-text text-[10px] leading-none tabular-nums">
            {unread}
          </span>
        )}
      </button>

      {/* 下拉体保持挂载、用 hidden 切换显隐(同账号 chip 的 keep-alive 模式)。 */}
      <div
        className={clsx(
          "absolute right-0 top-[calc(100%+6px)] w-[300px] bg-panel border border-titlebar shadow-raised rounded-none z-[200] p-[16px]",
          { hidden: !open },
        )}
      >
        <div className="text-[13px] text-strong font-display mb-[8px]">{t("notification.title")}</div>

        {(items ?? []).length > 0 ? (
          <div className="flex flex-col gap-[4px]">
            {(items ?? []).map((n, i) => (
              <div key={n.id ?? i} className="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
                {renderNotification(n)}
              </div>
            ))}
          </div>
        ) : (
          <p className="text-[12px] text-faint">{t("notification.empty")}</p>
        )}
      </div>
    </div>
  );
}

export default NotificationCenter;
