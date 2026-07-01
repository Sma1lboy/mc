import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { useAppStore, refreshKobeUser, refreshFriends } from "../store";
import { useAsync } from "../util/useAsync";
import { avatarTone, avatarInitial } from "../util/avatar";
import { t, useLang } from "../i18n";
import type { UserBrief } from "../ipc/bindings";

const INPUT =
  "h-[32px] px-[10px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full " +
  "placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";

const label = (u: UserBrief) => u.username || u.id.slice(0, 8);

/**
 * FriendsButton —— 顶栏好友入口(从账号 chip 拆出的独立按钮)。
 * 人像 chip + 好友数,点开保持挂载的下拉:搜索加好友 + 好友列表(在线点 + 活动行)。
 * 收到的好友请求改由通知中心(NotificationCenter)处理,这里不再展示。
 * 老/登录账号若还没用户名,先走兜底设名(新注册用户在注册时已设好,不会看到)。
 */
export function FriendsButton(): React.ReactElement {
  useLang();
  const [open, setOpen] = useState(false);
  const kobeUser = useAppStore((s) => s.kobeUser);
  const friends = useAppStore((s) => s.friends);
  const hasUsername = !!kobeUser?.username;
  const count = (friends ?? []).length;
  const onlineCount = (friends ?? []).filter((u) => u.online).length;
  const rootEl = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDoc = (e: MouseEvent) => {
      if (rootEl.current && !rootEl.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, []);

  return (
    <div ref={rootEl} className="relative">
      <button
        type="button"
        className="inline-flex items-center gap-[4px] h-[30px] px-[7px] rounded-none border-none bg-transparent text-dim text-[12px] cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-panel-2 hover:text-fg data-[state=open]:bg-panel-2 data-[state=open]:text-fg [-webkit-app-region:no-drag]"
        data-state={open ? "open" : "closed"}
        onClick={() => setOpen((o) => !o)}
        title={t("friend.title")}
      >
        <span className="grid place-items-center w-[16px] h-[16px] shrink-0">
          <Icon name="users" size={16} />
        </span>
        {count > 0 && <span className="min-w-[12px] text-center tabular-nums leading-none">{count}</span>}
      </button>

      {/* 下拉体保持挂载、用 hidden 切换显隐(不销毁重建):
          关掉再打开时好友列表来自 store 缓存、搜索框文字与滚动位置都保留,不再重拉。 */}
      <div
        className={clsx(
          "absolute right-0 top-[calc(100%+6px)] w-[300px] bg-panel border border-titlebar shadow-raised rounded-none z-[200] p-[16px]",
          { hidden: !open },
        )}
      >
        <div className="flex items-center justify-between mb-[10px]">
          <span className="text-[13px] text-strong font-display">{t("friend.title")}</span>
          {hasUsername && onlineCount > 0 && (
            <span className="text-[11px] text-accent bg-accent/15 px-[7px] py-[1px] tabular-nums">
              {t("friend.onlineCount", { n: onlineCount })}
            </span>
          )}
        </div>
        {hasUsername ? <Friends /> : <SetUsername />}
      </div>
    </div>
  );
}

/**
 * SetUsername —— 兜底设名:仅对没有用户名的老/登录账号显示。
 * 新注册用户在注册时即设好用户名(store.kobeSignup),正常流程不会走到这里。
 */
function SetUsername(): React.ReactElement {
  useLang();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  async function save() {
    const n = name.trim();
    if (busy || n.length < 3) return;
    setBusy(true);
    try {
      await api.friendSetUsername(n);
      await refreshKobeUser();
      toast({ type: "success", message: t("friend.usernameSaved") });
    } catch {
      toast({ type: "error", message: t("friend.usernameError") });
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-[8px]">
      <p className="text-[12px] text-muted leading-[1.5]">{t("friend.setUsernameHint")}</p>
      <input
        className={INPUT}
        type="text"
        maxLength={24}
        placeholder={t("friend.usernamePlaceholder")}
        value={name}
        onChange={(e) => setName(e.currentTarget.value)}
        onKeyDown={(e) => e.key === "Enter" && void save()}
      />
      <Button variant="primary" disabled={busy || name.trim().length < 3} onClick={() => void save()} className="w-full justify-center">
        {t("friend.saveUsername")}
      </Button>
    </div>
  );
}

function Friends(): React.ReactElement {
  useLang();
  // 好友列表来自 store(单一真相 + 连续 30s 轮询),不再起各自的 resource/轮询。
  const friends = useAppStore((s) => s.friends);
  const [query, setQuery] = useState("");
  const q = query.trim();
  const { data: results, loading: searching, refetch: refetchSearch } = useAsync(
    () => (q.length >= 2 ? api.friendSearch(q) : Promise.resolve([] as UserBrief[])),
    [q],
  );
  const [busy, setBusy] = useState(false);

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

  const sendRequest = (u: UserBrief) =>
    act(async () => {
      await api.friendRequest(u.id);
      toast({ type: "success", message: t("friend.requestSent") });
      refetchSearch();
      void refreshFriends();
    });
  const remove = (u: UserBrief) =>
    act(async () => {
      await api.friendRemove(u.id);
      void refreshFriends();
    });

  // 状态分段:全部 / 在线(只在线时一眼看谁能一起玩)。
  const [tab, setTab] = useState<"all" | "online">("all");
  const all = friends ?? [];
  const onlineList = all.filter((u) => u.online);
  const shown = tab === "online" ? onlineList : all;

  return (
    <div className="flex flex-col gap-[10px]">
      {/* 状态分段控件 */}
      <div className="flex bg-panel-2 shadow-input p-[2px]">
        <button
          type="button"
          className={clsx(
            "flex-1 h-[24px] border-none cursor-pointer text-[12px] tabular-nums transition-[background-color,color] duration-[var(--dur)] ease-app",
            tab === "all" ? "bg-accent text-accent-text shadow-raised font-medium" : "bg-transparent text-muted hover:text-fg",
          )}
          onClick={() => setTab("all")}
        >
          {t("friend.all")} {all.length}
        </button>
        <button
          type="button"
          className={clsx(
            "flex-1 h-[24px] border-none cursor-pointer text-[12px] tabular-nums transition-[background-color,color] duration-[var(--dur)] ease-app",
            tab === "online" ? "bg-accent text-accent-text shadow-raised font-medium" : "bg-transparent text-muted hover:text-fg",
          )}
          onClick={() => setTab("online")}
        >
          {t("friend.online")} {onlineList.length}
        </button>
      </div>

      {/* 加好友:搜索用户名 */}
      <div className="flex flex-col gap-[6px]">
        <input
          className={INPUT}
          type="text"
          placeholder={t("friend.addPlaceholder")}
          value={query}
          onChange={(e) => setQuery(e.currentTarget.value)}
        />
        {q.length >= 2 && (
          <div className="flex flex-col gap-[4px] max-h-[140px] overflow-y-auto">
            {(results ?? []).length > 0 ? (
              (results ?? []).map((u) => (
                <div key={u.id} className="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
                  <span className="text-[13px] text-fg truncate flex-1">{label(u)}</span>
                  <button
                    className="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                    disabled={busy}
                    onClick={() => void sendRequest(u)}
                  >
                    {t("friend.add")}
                  </button>
                </div>
              ))
            ) : (
              <p className="text-[12px] text-faint px-[2px]">{searching ? t("friend.searching") : t("friend.noResults")}</p>
            )}
          </div>
        )}
      </div>

      {/* 好友列表(头像方块瓦片 + 角落状态 pip;在线行实色、离线行灰)。 */}
      {all.length > 0 ? (
        shown.length > 0 ? (
          <div className="flex flex-col gap-[1px] max-h-[300px] overflow-y-auto">
            {shown.map((u) => (
              <div
                key={u.id}
                className="flex items-center gap-[9px] px-[6px] py-[5px] group hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app"
              >
                <span
                  className={clsx(
                    "relative w-[24px] h-[24px] shrink-0 grid place-items-center shadow-raised font-display text-[12px] text-[#1a1b12]",
                    { "grayscale brightness-75": !u.online },
                  )}
                  style={{ backgroundColor: avatarTone(label(u)) }}
                  aria-hidden="true"
                >
                  {avatarInitial(label(u))}
                  <span
                    className={clsx(
                      "absolute -right-[2px] -bottom-[2px] w-[8px] h-[8px] shadow-[0_0_0_2px_var(--color-panel)]",
                      u.online ? "bg-accent" : "bg-faint",
                    )}
                  />
                </span>
                <div className="flex flex-col min-w-0 flex-1">
                  <span className={clsx("text-[13px] truncate", u.online ? "text-fg" : "text-muted")}>{label(u)}</span>
                  <span
                    className={clsx("text-[11px] truncate", u.online && !!u.activity ? "text-accent" : "text-faint")}
                  >
                    {u.online
                      ? u.activity
                        ? t("friend.playing", { name: u.activity })
                        : t("friend.idle")
                      : t("friend.offline")}
                  </span>
                </div>
                <button
                  className="text-[11px] text-danger-text hover:underline bg-transparent border-none cursor-pointer opacity-0 group-hover:opacity-100 disabled:opacity-50"
                  disabled={busy}
                  onClick={() => void remove(u)}
                  title={t("friend.remove")}
                >
                  {t("friend.remove")}
                </button>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-[12px] text-faint px-[2px] py-[6px]">{t("friend.noneOnline")}</p>
        )
      ) : (
        <div className="flex flex-col items-center gap-[8px] py-[18px] text-center">
          <span className="w-[34px] h-[34px] grid grid-rows-[11px_1fr] shadow-input overflow-hidden" aria-hidden="true">
            <span className="bg-accent" />
            <span className="bg-[#7a5b3a]" />
          </span>
          <p className="text-[12px] text-faint leading-[1.6] max-w-[200px]">{t("friend.noFriends")}</p>
        </div>
      )}
    </div>
  );
}

export default FriendsButton;
