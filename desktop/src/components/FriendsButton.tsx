import { Component, createResource, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { Button } from "./Button";
import { Icon } from "./Icon";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import {
  kobeUser,
  refreshKobeUser,
  friends,
  refreshFriends,
} from "../store";
import { avatarTone, avatarInitial } from "../util/avatar";
import { t } from "../i18n";
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
export const FriendsButton: Component = () => {
  const [open, setOpen] = createSignal(false);
  const hasUsername = () => !!kobeUser()?.username;
  const count = () => (friends() ?? []).length;
  const onlineCount = () => (friends() ?? []).filter((u) => u.online).length;
  let rootEl: HTMLDivElement | undefined;

  onMount(() => {
    const onDoc = (e: MouseEvent) => {
      if (rootEl && !rootEl.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    onCleanup(() => document.removeEventListener("mousedown", onDoc));
  });

  return (
    <div ref={rootEl} class="relative">
      <button
        type="button"
        class="inline-flex items-center gap-[4px] h-[30px] px-[7px] rounded-none border-none bg-transparent text-dim text-[12px] cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-panel-2 hover:text-fg data-[state=open]:bg-panel-2 data-[state=open]:text-fg [-webkit-app-region:no-drag]"
        data-state={open() ? "open" : "closed"}
        onClick={() => setOpen((o) => !o)}
        title={t("friend.title")}
      >
        <span class="grid place-items-center w-[16px] h-[16px] shrink-0">
          <Icon name="users" size={16} />
        </span>
        <Show when={count() > 0}>
          <span class="min-w-[12px] text-center tabular-nums leading-none">{count()}</span>
        </Show>
      </button>

      {/* 下拉体保持挂载、用 hidden 切换显隐(不 <Show> 销毁重建):
          关掉再打开时好友列表来自 store 缓存、搜索框文字与滚动位置都保留,不再重拉。 */}
      <div
        class="absolute right-0 top-[calc(100%+6px)] w-[300px] bg-panel border border-titlebar shadow-raised rounded-none z-[200] p-[16px]"
        classList={{ hidden: !open() }}
      >
        <div class="flex items-center justify-between mb-[10px]">
          <span class="text-[13px] text-strong font-display">{t("friend.title")}</span>
          <Show when={hasUsername() && onlineCount() > 0}>
            <span class="text-[11px] text-accent bg-accent/15 px-[7px] py-[1px] tabular-nums">
              {t("friend.onlineCount", { n: onlineCount() })}
            </span>
          </Show>
        </div>
        <Show when={hasUsername()} fallback={<SetUsername />}>
          <Friends />
        </Show>
      </div>
    </div>
  );
};

/**
 * SetUsername —— 兜底设名:仅对没有用户名的老/登录账号显示。
 * 新注册用户在注册时即设好用户名(store.kobeSignup),正常流程不会走到这里。
 */
const SetUsername: Component = () => {
  const [name, setName] = createSignal("");
  const [busy, setBusy] = createSignal(false);

  async function save() {
    const n = name().trim();
    if (busy() || n.length < 3) return;
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
    <div class="flex flex-col gap-[8px]">
      <p class="text-[12px] text-muted leading-[1.5]">{t("friend.setUsernameHint")}</p>
      <input
        class={INPUT}
        type="text"
        maxLength={24}
        placeholder={t("friend.usernamePlaceholder")}
        value={name()}
        onInput={(e) => setName(e.currentTarget.value)}
        onKeyDown={(e) => e.key === "Enter" && void save()}
      />
      <Button variant="primary" disabled={busy() || name().trim().length < 3} onClick={() => void save()} class="w-full justify-center">
        {t("friend.saveUsername")}
      </Button>
    </div>
  );
};

const Friends: Component = () => {
  // 好友列表来自 store(单一真相 + 连续 30s 轮询),不再起各自的 resource/轮询。
  const [query, setQuery] = createSignal("");
  const [results, { refetch: refetchSearch }] = createResource(
    () => query().trim(),
    (q) => (q.length >= 2 ? api.friendSearch(q) : Promise.resolve([] as UserBrief[])),
  );
  const [busy, setBusy] = createSignal(false);

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

  const sendRequest = (u: UserBrief) =>
    act(async () => {
      await api.friendRequest(u.id);
      toast({ type: "success", message: t("friend.requestSent") });
      void refetchSearch();
      void refreshFriends();
    });
  const remove = (u: UserBrief) =>
    act(async () => {
      await api.friendRemove(u.id);
      void refreshFriends();
    });

  // 状态分段:全部 / 在线(只在线时一眼看谁能一起玩)。
  const [tab, setTab] = createSignal<"all" | "online">("all");
  const all = () => friends() ?? [];
  const onlineList = () => all().filter((u) => u.online);
  const shown = () => (tab() === "online" ? onlineList() : all());

  return (
    <div class="flex flex-col gap-[10px]">
      {/* 状态分段控件 */}
      <div class="flex bg-panel-2 shadow-input p-[2px]">
        <button
          type="button"
          class="flex-1 h-[24px] border-none cursor-pointer text-[12px] tabular-nums transition-[background-color,color] duration-[var(--dur)] ease-app"
          classList={{ "bg-accent text-accent-text shadow-raised font-medium": tab() === "all", "bg-transparent text-muted hover:text-fg": tab() !== "all" }}
          onClick={() => setTab("all")}
        >
          {t("friend.all")} {all().length}
        </button>
        <button
          type="button"
          class="flex-1 h-[24px] border-none cursor-pointer text-[12px] tabular-nums transition-[background-color,color] duration-[var(--dur)] ease-app"
          classList={{ "bg-accent text-accent-text shadow-raised font-medium": tab() === "online", "bg-transparent text-muted hover:text-fg": tab() !== "online" }}
          onClick={() => setTab("online")}
        >
          {t("friend.online")} {onlineList().length}
        </button>
      </div>

      {/* 加好友:搜索用户名 */}
      <div class="flex flex-col gap-[6px]">
        <input
          class={INPUT}
          type="text"
          placeholder={t("friend.addPlaceholder")}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        <Show when={query().trim().length >= 2}>
          <div class="flex flex-col gap-[4px] max-h-[140px] overflow-y-auto">
            <Show
              when={(results() ?? []).length > 0}
              fallback={<p class="text-[12px] text-faint px-[2px]">{results.loading ? t("friend.searching") : t("friend.noResults")}</p>}
            >
              <For each={results()}>
                {(u) => (
                  <div class="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
                    <span class="text-[13px] text-fg truncate flex-1">{label(u)}</span>
                    <button
                      class="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                      disabled={busy()}
                      onClick={() => void sendRequest(u)}
                    >
                      {t("friend.add")}
                    </button>
                  </div>
                )}
              </For>
            </Show>
          </div>
        </Show>
      </div>

      {/* 好友列表(头像方块瓦片 + 角落状态 pip;在线行实色、离线行灰)。 */}
      <Show
        when={all().length > 0}
        fallback={
          <div class="flex flex-col items-center gap-[8px] py-[18px] text-center">
            <span class="w-[34px] h-[34px] grid grid-rows-[11px_1fr] shadow-input overflow-hidden" aria-hidden="true">
              <span class="bg-accent" />
              <span class="bg-[#7a5b3a]" />
            </span>
            <p class="text-[12px] text-faint leading-[1.6] max-w-[200px]">{t("friend.noFriends")}</p>
          </div>
        }
      >
        <Show
          when={shown().length > 0}
          fallback={<p class="text-[12px] text-faint px-[2px] py-[6px]">{t("friend.noneOnline")}</p>}
        >
          <div class="flex flex-col gap-[1px] max-h-[300px] overflow-y-auto">
            <For each={shown()}>
              {(u) => (
                <div class="flex items-center gap-[9px] px-[6px] py-[5px] group hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app">
                  <span
                    class="relative w-[24px] h-[24px] shrink-0 grid place-items-center shadow-raised font-display text-[12px] text-[#1a1b12]"
                    classList={{ "grayscale brightness-75": !u.online }}
                    style={{ "background-color": avatarTone(label(u)) }}
                    aria-hidden="true"
                  >
                    {avatarInitial(label(u))}
                    <span
                      class="absolute -right-[2px] -bottom-[2px] w-[8px] h-[8px] shadow-[0_0_0_2px_var(--color-panel)]"
                      classList={{ "bg-accent": u.online, "bg-faint": !u.online }}
                    />
                  </span>
                  <div class="flex flex-col min-w-0 flex-1">
                    <span class="text-[13px] truncate" classList={{ "text-fg": u.online, "text-muted": !u.online }}>
                      {label(u)}
                    </span>
                    <span
                      class="text-[11px] truncate"
                      classList={{ "text-accent": u.online && !!u.activity, "text-faint": !(u.online && u.activity) }}
                    >
                      {u.online
                        ? u.activity
                          ? t("friend.playing", { name: u.activity })
                          : t("friend.idle")
                        : t("friend.offline")}
                    </span>
                  </div>
                  <button
                    class="text-[11px] text-danger-text hover:underline bg-transparent border-none cursor-pointer opacity-0 group-hover:opacity-100 disabled:opacity-50"
                    disabled={busy()}
                    onClick={() => void remove(u)}
                    title={t("friend.remove")}
                  >
                    {t("friend.remove")}
                  </button>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
};
