import { Component, createResource, createSignal, For, Show } from "solid-js";
import { Button } from "./Button";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { kobeUser, refreshKobeUser } from "../store";
import { t } from "../i18n";
import type { UserBrief } from "../ipc/bindings";

const INPUT =
  "h-[32px] px-[10px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full " +
  "placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";

const label = (u: UserBrief) => u.username || u.id.slice(0, 8);

/**
 * FriendsSection —— kobeMC 账号下拉里的好友区:先要求设用户名(好友靠用户名搜),
 * 然后搜索加好友、处理收到的请求、查看/删除好友。
 */
export const FriendsSection: Component = () => {
  const hasUsername = () => !!kobeUser()?.username;

  return (
    <div class="mt-[12px] pt-[12px] border-t border-titlebar">
      <div class="text-[13px] text-strong font-display mb-[8px]">{t("friend.title")}</div>
      <Show when={hasUsername()} fallback={<SetUsername />}>
        <Friends />
      </Show>
    </div>
  );
};

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
  const [friends, { refetch: refetchFriends }] = createResource(() => api.friendList());
  const [requests, { refetch: refetchRequests }] = createResource(() => api.friendRequests());
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
      void refetchFriends();
    });
  const accept = (u: UserBrief) =>
    act(async () => {
      await api.friendAccept(u.id);
      void refetchRequests();
      void refetchFriends();
    });
  const decline = (u: UserBrief) =>
    act(async () => {
      await api.friendDecline(u.id);
      void refetchRequests();
    });
  const remove = (u: UserBrief) =>
    act(async () => {
      await api.friendRemove(u.id);
      void refetchFriends();
    });

  return (
    <div class="flex flex-col gap-[10px]">
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

      {/* 收到的请求 */}
      <Show when={(requests() ?? []).length > 0}>
        <div class="flex flex-col gap-[4px]">
          <div class="text-[11px] text-muted">{t("friend.requests")}</div>
          <For each={requests()}>
            {(u) => (
              <div class="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
                <span class="text-[13px] text-fg truncate flex-1">{label(u)}</span>
                <button class="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50" disabled={busy()} onClick={() => void accept(u)}>
                  {t("friend.accept")}
                </button>
                <button class="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50" disabled={busy()} onClick={() => void decline(u)}>
                  {t("friend.decline")}
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* 好友列表 */}
      <div class="flex flex-col gap-[4px]">
        <Show
          when={(friends() ?? []).length > 0}
          fallback={<p class="text-[12px] text-faint">{friends.loading ? "" : t("friend.noFriends")}</p>}
        >
          <For each={friends()}>
            {(u) => (
              <div class="flex items-center gap-[8px] px-[2px] py-[3px] group">
                <span class="w-[6px] h-[6px] bg-accent shrink-0" aria-hidden="true" />
                <span class="text-[13px] text-fg truncate flex-1">{label(u)}</span>
                <button
                  class="text-[11px] text-danger-text hover:underline bg-transparent border-none cursor-pointer opacity-0 group-hover:opacity-100 disabled:opacity-50"
                  disabled={busy()}
                  onClick={() => void remove(u)}
                >
                  {t("friend.remove")}
                </button>
              </div>
            )}
          </For>
        </Show>
      </div>
    </div>
  );
};
