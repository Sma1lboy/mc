import { api } from "../ipc/api";
import { toast } from "../components/Toast";
import { t } from "../i18n";
import type { AuthUser } from "../ipc/bindings";
import { get, set } from "./state";

export const kobeUser = (): AuthUser | null => get().kobeUser;

/** 是否已登录 kobeMC 账号(快照)。 */
export function isKobeSignedIn(): boolean {
  return get().kobeUser !== null;
}

// ===== 在线状态心跳(presence) =====
/** 当前活动文案:取任一正在运行的实例名;无则 null(空闲)。 */
function currentActivity(): string | null {
  const running = get().runningIds;
  if (running.size === 0) return null;
  const inst = (get().instances ?? []).find((x) => running.has(x.id));
  return inst?.name ?? null;
}

/** 上报一次心跳(仅在已登录时)。 */
export function sendPresenceHeartbeat(): void {
  if (!isKobeSignedIn()) return;
  void api.presenceHeartbeat(currentActivity()).catch(() => {});
}

if (typeof window !== "undefined") {
  setInterval(sendPresenceHeartbeat, 60_000);
}

/** 邮箱 + 密码登录 kobeMC 账号;成功后填充全局会话。 */
export async function kobeLogin(email: string, password: string): Promise<void> {
  const user = await api.kobemcLogin(email, password);
  set({ kobeUser: user });
  sendPresenceHeartbeat();
  toast({ type: "info", message: t("kobe.toast.loggedIn", { name: kobeDisplayName(user) }) });
}

/**
 * 注册新 kobeMC 账号(注册即登录,沿用同一会话 cookie)。
 * 设好友用户名失败(如已被占用)不阻断登录:toast 提示后照常保持登录态。
 */
export async function kobeSignup(email: string, password: string, username: string): Promise<void> {
  const user = await api.kobemcSignup(email, password, username);
  set({ kobeUser: user });
  sendPresenceHeartbeat();
  toast({ type: "info", message: t("kobe.toast.signedUp", { name: kobeDisplayName(user) }) });
  try {
    await api.friendSetUsername(username);
  } catch {
    toast({ type: "error", message: t("kobe.usernameTaken") });
  }
  await refreshKobeUser();
}

/** 退出 kobeMC 账号(清后端会话 + 本地状态)。 */
export async function kobeLogout(): Promise<void> {
  const email = get().kobeUser?.email ?? null;
  try {
    await api.kobemcLogout();
  } finally {
    set({ kobeUser: null });
  }
  // 显式退出后下次启动不再自动登录该账号;凭据仍留在列表里(只关掉它的 auto_login)。
  if (email) {
    try {
      await api.kobeSetAutoLogin(email, false);
    } catch {
      /* 忽略 */
    }
  }
}

/** 账号展示名:优先昵称 → 用户名 → 邮箱 → id 前缀。 */
export function kobeDisplayName(user: AuthUser): string {
  return user.name || user.username || user.email || user.id.slice(0, 8);
}

/** 重新拉取后端会话用户(设用户名/资料变更后刷新 kobeUser)。 */
export async function refreshKobeUser(): Promise<void> {
  try {
    set({ kobeUser: (await api.kobemcSession()) ?? null });
  } catch {
    /* 会话探测失败不影响现有登录态 */
  }
}

// 启动时探一次后端会话:有效则恢复登录态;否则(全新进程)若记住凭据且开了自动登录,静默登录。
if (typeof window !== "undefined") {
  void api
    .kobemcSession()
    .then(async (user) => {
      if (user) {
        set({ kobeUser: user });
        sendPresenceHeartbeat();
        return;
      }
      try {
        const auto = (await api.kobeListCredentials()).find((c) => c.auto_login);
        if (auto?.email && auto.password) {
          await kobeLogin(auto.email, auto.password);
        }
      } catch {
        /* 自动登录尽力而为:失败(密码改了 / 网络)保持登出态,不打扰 */
      }
    })
    .catch(() => {});
}

// ===== 社交 UI 可见性(kobeMC 账号 / 领域 / 好友) =====
export const socialEnabled = (): boolean => get().socialEnabled;

if (typeof window !== "undefined") {
  void api
    .socialEnabled()
    .then((on) => set({ socialEnabled: on }))
    .catch(() => {});
}

/** 设置社交 UI 可见性并持久化(写 social_enabled 显式覆盖)。 */
export async function setSocialEnabled(on: boolean): Promise<void> {
  set({ socialEnabled: on });
  try {
    const s = await api.getSettings();
    await api.setSettings({ ...s, social_enabled: on });
  } catch {
    /* 持久化失败不影响本次会话的显示状态 */
  }
}
