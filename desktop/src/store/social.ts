import { api } from "../ipc/api";
import type { UserBrief, Identity, Notification } from "../ipc/bindings";
import { get, set } from "./state";
import { isKobeSignedIn } from "./kobe";

// ===== 社交数据缓存(好友 / 好友请求 / 关联身份 / 通知) =====
export const friends = (): UserBrief[] => get().friends;
export const friendRequests = (): UserBrief[] => get().friendRequests;
export const accountIdentities = (): Identity[] => get().accountIdentities;
export const notifications = (): Notification[] => get().notifications;

/** 刷新好友列表(含在线状态/活动);未登录或社交关闭时不动。 */
export async function refreshFriends(): Promise<void> {
  if (!isKobeSignedIn() || !get().socialEnabled) return;
  try {
    set({ friends: await api.friendList() });
  } catch {
    /* 保留旧值 */
  }
}

/** 刷新收到的好友请求;未登录或社交关闭时不动。 */
export async function refreshFriendRequests(): Promise<void> {
  if (!isKobeSignedIn() || !get().socialEnabled) return;
  try {
    set({ friendRequests: await api.friendRequests() });
  } catch {
    /* 保留旧值 */
  }
}

/** 刷新通知中心列表;未登录或社交关闭时不动。容错:失败保留旧值。 */
export async function refreshNotifications(): Promise<void> {
  if (!isKobeSignedIn() || !get().socialEnabled) return;
  try {
    set({ notifications: await api.notifications() });
  } catch {
    /* 保留旧值 */
  }
}

/** 标记所有通知为已读(打开铃铛下拉时调),随后刷新一次清空角标。 */
export async function markNotificationsRead(): Promise<void> {
  try {
    await api.notificationsRead();
  } catch {
    /* 标记失败不阻断:下次刷新仍按服务端状态显示 */
  }
  await refreshNotifications();
}

/** 刷新当前 kobeMC 账号的关联身份;未登录时不动。 */
export async function refreshIdentities(): Promise<void> {
  if (!isKobeSignedIn()) return;
  try {
    set({ accountIdentities: await api.accountIdentities() });
  } catch {
    /* 保留旧值 */
  }
}
