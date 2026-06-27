import {
  Component,
  createEffect,
  createResource,
  createSignal,
  For,
  onCleanup,
  Show,
} from "solid-js";
import { Button } from "./Button";
import { Panel } from "./Panel";
import { Heading } from "./Typography";
import { Dialog } from "./Dialog";
import { Select } from "./Select";
import { Spinner } from "./Spinner";
import { Tag } from "./Tag";
import { Toggle } from "./Toggle";
import { toast } from "./Toast";
import { api, onRealmSyncProgress } from "../ipc/api";
import { activeRoot, refreshInstances, kobeUser, isKobeSignedIn, setCurrentPage, socialEnabled } from "../store";
import { t } from "../i18n";
import type { InstanceSummary, RealmMember, SyncReport } from "../ipc/bindings";

/** role 字符串 → 本地化标签。 */
function roleLabel(role: string): string {
  return role === "owner"
    ? t("realm.roleOwner")
    : role === "admin"
      ? t("realm.roleAdmin")
      : t("realm.roleMember");
}

/**
 * RealmPanel —— 实例详情里的「领域」段。把领域完全收进 instance 入口:
 * - 非领域实例:一个「分享为领域」入口。
 * - 已加入但未装核心(pending):一个「开始同步(Begin)」按钮 —— 装版本/loader + 下 mods。
 * - 已装核心的领域实例:加入码 / 成员 / 自动检测+同步 / 推送清单(owner·admin)/ 退出·解散。
 */
export const RealmPanel: Component<{ instance: InstanceSummary; onChanged?: () => void }> = (props) => {
  const realm = () => props.instance.realm;

  return (
    <div class="px-[28px] py-[14px]">
      <Show when={realm()} fallback={<ShareEntry instance={props.instance} onChanged={props.onChanged} />}>
        <Show
          when={props.instance.installed}
          fallback={<BeginEntry instance={props.instance} onChanged={props.onChanged} />}
        >
          <RealmManage instance={props.instance} onChanged={props.onChanged} />
        </Show>
      </Show>
    </div>
  );
};

/* ---------- 非领域实例:分享入口 ---------- */

const ShareEntry: Component<{ instance: InstanceSummary; onChanged?: () => void }> = (props) => {
  const [open, setOpen] = createSignal(false);
  return (
    <Panel variant="sunken" class="p-[16px] flex items-center justify-between gap-[12px] flex-wrap">
      <div class="min-w-0">
        <div class="flex items-center gap-[8px]">
          <Heading size="sub" as="h3" class="m-0 text-[14px]">
            {t("realm.title")}
          </Heading>
        </div>
        <p class="text-[12px] text-muted mt-[4px] leading-[1.6]">{t("realm.shareHint")}</p>
      </div>
      <Button variant="ghost" onClick={() => setOpen(true)}>
        {t("realm.shareAction")}
      </Button>
      <ShareDialog instance={props.instance} open={open()} onClose={() => setOpen(false)} onShared={props.onChanged} />
    </Panel>
  );
};

const ShareDialog: Component<{
  instance: InstanceSummary;
  open: boolean;
  onClose: () => void;
  onShared?: () => void;
}> = (props) => {
  const [name, setName] = createSignal(props.instance.name);
  const [expiry, setExpiry] = createSignal("0");
  const [busy, setBusy] = createSignal(false);

  async function submit() {
    if (busy() || !name().trim()) return;
    if (!isKobeSignedIn()) {
      toast({ type: "error", message: t("realm.needLogin") });
      return;
    }
    setBusy(true);
    try {
      const secs = parseInt(expiry(), 10) || 0;
      const r = await api.realmCreate(
        activeRoot(),
        props.instance.id,
        name().trim(),
        props.instance.mc_version,
        props.instance.loader ?? "vanilla",
        props.instance.loader_version ?? null,
        secs > 0 ? secs : null,
      );
      toast({ type: "success", message: t("realm.createdToast", { name: r.name }) });
      props.onClose();
      props.onShared?.();
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={props.open} onClose={props.onClose} label={t("realm.shareTitle")}>
      <div class="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub" as="h2" class="m-0">
          {t("realm.shareTitle")}
        </Heading>
        <Show when={!isKobeSignedIn()}>
          <p class="text-[12px] text-danger-text">{t("realm.needLogin")}</p>
        </Show>
        <label class="flex flex-col gap-[6px]">
          <span class="text-[12px] text-muted">{t("realm.nameLabel")}</span>
          <input
            class="h-[38px] px-[14px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            type="text"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
          />
        </label>
        <label class="flex flex-col gap-[6px]">
          <span class="text-[12px] text-muted">{t("realm.expiry")}</span>
          <Select
            value={expiry()}
            onChange={setExpiry}
            options={[
              { value: "0", label: t("realm.expiryNever") },
              { value: "86400", label: t("realm.expiry1d") },
              { value: "604800", label: t("realm.expiry7d") },
              { value: "2592000", label: t("realm.expiry30d") },
            ]}
          />
        </label>
        <div class="flex justify-end gap-[8px] mt-[4px]">
          <Button variant="ghost" onClick={props.onClose}>
            {t("realm.cancel")}
          </Button>
          <Button variant="primary" disabled={busy() || !name().trim()} onClick={() => void submit()}>
            {busy() ? t("realm.creating") : t("realm.shareAction")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
};

/* ---------- pending:开始同步 ---------- */

const BeginEntry: Component<{ instance: InstanceSummary; onChanged?: () => void }> = (props) => {
  const r = () => props.instance.realm!;
  const [busy, setBusy] = createSignal(false);
  const [progress, setProgress] = createSignal<{ current: number; total: number } | null>(null);
  onCleanup(onRealmSyncProgress((p) => setProgress({ current: p.current, total: p.total })));

  async function begin() {
    if (busy()) return;
    setBusy(true);
    setProgress({ current: 0, total: 0 });
    try {
      const report: SyncReport = await api.realmBegin(r().realm_id, activeRoot(), props.instance.id);
      refreshInstances();
      props.onChanged?.();
      toast({
        type: report.failed.length ? "error" : "success",
        message: report.failed.length
          ? t("realm.syncFailed", { count: report.failed.length })
          : t("realm.beginDone"),
      });
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  return (
    <Panel variant="sunken" class="p-[20px] flex flex-col gap-[12px]">
      <div class="flex items-center gap-[8px] flex-wrap">
        <Heading size="sub" as="h3" class="m-0 text-[14px]">
          {r().name || t("realm.title")}
        </Heading>
        <Tag>{roleLabel(r().role)}</Tag>
        <Show when={r().code}>
          <span class="font-mono text-[12px] text-accent tracking-[0.12em]">{r().code}</span>
        </Show>
      </div>
      <p class="text-[12px] text-muted leading-[1.6]">{t("realm.beginHint")}</p>
      <Button variant="primary" class="self-start" disabled={busy()} onClick={() => void begin()}>
        {busy() ? t("realm.syncing") : t("realm.beginAction")}
      </Button>
      <Show when={progress()}>
        {(pr) => (
          <div class="h-[6px] w-full bg-window shadow-input rounded-none overflow-hidden">
            <div
              class="h-full bg-accent transition-[width] duration-150 ease-app"
              style={{ width: `${pr().total > 0 ? Math.round((pr().current / pr().total) * 100) : 8}%` }}
            />
          </div>
        )}
      </Show>
    </Panel>
  );
};

/* ---------- 已装核心的领域实例:管理 ---------- */

const RealmManage: Component<{ instance: InstanceSummary; onChanged?: () => void }> = (props) => {
  const rid = () => props.instance.realm!.realm_id;
  const myId = () => kobeUser()?.id;

  const [summary, { refetch: refetchSummary }] = createResource(rid, () => api.realmGet(rid()));
  const [members, { refetch: refetchMembers }] = createResource(rid, () => api.realmMembers(rid()));
  const role = () => summary()?.role ?? props.instance.realm!.role;
  const isOwner = () => role() === "owner";
  const canPush = () => role() === "owner" || role() === "admin";

  // 好友列表(仅在社交开启 + 已登录时拉取):用于「邀请好友」与成员的好友标记。
  const [friends, { refetch: refetchFriends }] = createResource(
    () => (socialEnabled() && isKobeSignedIn() ? rid() : null),
    () => api.friendList(),
  );
  const memberIds = () => new Set((members() ?? []).map((m) => m.user_id));
  const friendIds = () => new Set((friends() ?? []).map((f) => f.id));

  // 面板打开期间周期性刷新好友在线状态/活动(节流到 30s),关闭即停。
  const friendsPoll = setInterval(() => {
    if (socialEnabled() && isKobeSignedIn()) void refetchFriends();
  }, 30_000);
  onCleanup(() => clearInterval(friendsPoll));

  // 在线好友优先,其次按用户名排序,便于优先邀请在线好友。
  const sortedFriends = () =>
    [...(friends() ?? [])].sort((a, b) => {
      if (!!a.online !== !!b.online) return a.online ? -1 : 1;
      return (a.username || a.id).localeCompare(b.username || b.id);
    });

  const [removeExtras, setRemoveExtras] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [progress, setProgress] = createSignal<{ current: number; total: number } | null>(null);
  const [confirmKind, setConfirmKind] = createSignal<"leave" | "disband" | null>(null);

  // 自动检测差异:随 (领域, 清单版本) 自动重算。
  const [plan, { refetch: refetchPlan }] = createResource(
    () => {
      const s = summary();
      return s ? { rid: rid(), iid: props.instance.id, mv: s.manifest_version } : null;
    },
    (k) => api.realmPlanSync(k.rid, activeRoot(), k.iid),
  );

  onCleanup(onRealmSyncProgress((p) => setProgress({ current: p.current, total: p.total })));

  function fail(e: unknown) {
    toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
  }

  // 纯新增差异自动同步;有移除(破坏性)留给显式确认。去重避免死循环。
  let autoKey = "";
  createEffect(() => {
    const p = plan();
    if (!p || busy()) return;
    const key = `${rid()}:${p.version}`;
    if (p.download.length > 0 && p.remove.length === 0 && autoKey !== key) {
      autoKey = key;
      void runSync(false);
    }
  });

  async function runSync(remove: boolean) {
    if (busy()) return;
    setBusy(true);
    setProgress({ current: 0, total: 0 });
    try {
      const report: SyncReport = await api.realmSync(rid(), activeRoot(), props.instance.id, remove);
      refreshInstances();
      void refetchMembers();
      await refetchPlan();
      setRemoveExtras(false);
      toast({
        type: report.failed.length ? "error" : "success",
        message: report.failed.length
          ? t("realm.syncFailed", { count: report.failed.length })
          : t("realm.syncDone", { downloaded: report.downloaded, removed: report.removed }),
      });
      if (report.manual.length) {
        toast({ type: "info", message: t("realm.manualCount", { count: report.manual.length }) });
      }
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  async function pushManifest() {
    setBusy(true);
    try {
      const version = await api.realmPushManifest(
        rid(),
        activeRoot(),
        props.instance.id,
        props.instance.mc_version,
        props.instance.loader ?? "vanilla",
        props.instance.loader_version ?? null,
      );
      await refetchSummary();
      await refetchPlan();
      toast({ type: "success", message: t("realm.pushDone", { version }) });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function copyCode() {
    const code = summary()?.code ?? props.instance.realm!.code;
    if (!code) return;
    try {
      await navigator.clipboard.writeText(code);
      toast({ type: "success", message: t("realm.copied") });
    } catch (e) {
      fail(e);
    }
  }

  async function setRole(uid: string, r: string) {
    setBusy(true);
    try {
      await api.realmSetRole(rid(), uid, r);
      await refetchMembers();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function removeMember(uid: string) {
    setBusy(true);
    try {
      await api.realmRemoveMember(rid(), uid);
      await refetchMembers();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function invite(uid: string) {
    if (busy()) return;
    setBusy(true);
    try {
      await api.realmInvite(rid(), uid);
      await refetchMembers();
      toast({ type: "success", message: t("realm.inviteDone") });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function doLeave() {
    if (!myId()) return;
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmLeave(rid(), myId()!, activeRoot(), props.instance.id);
      refreshInstances();
      props.onChanged?.();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  async function doDisband() {
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmDisband(rid(), activeRoot(), props.instance.id);
      refreshInstances();
      props.onChanged?.();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  const code = () => summary()?.code ?? props.instance.realm!.code ?? "";

  return (
    <div class="flex flex-col gap-[12px]">
      {/* 头部:加入码 + 角色 + 清单版本 */}
      <Panel variant="sunken" class="p-[16px] flex items-center justify-between gap-[12px] flex-wrap">
        <div class="flex items-center gap-[8px] flex-wrap min-w-0">
          <Heading size="sub" as="h3" class="m-0 text-[14px]">
            {summary()?.name ?? props.instance.realm!.name ?? t("realm.title")}
          </Heading>
          <Tag>{roleLabel(role())}</Tag>
          <Show when={summary()}>
            <span class="text-[11px] text-muted bg-window shadow-input px-[6px] py-[2px]">
              {t("realm.manifestVersion", { version: summary()!.manifest_version })}
            </span>
          </Show>
        </div>
        <button
          type="button"
          class="font-mono text-[16px] text-accent tracking-[0.16em] bg-window shadow-input px-[12px] py-[5px] cursor-pointer hover:brightness-110"
          title={t("realm.copyCode")}
          onClick={() => void copyCode()}
        >
          {code()}
        </button>
      </Panel>

      {/* 同步状态(自动检测 + 自动同步;破坏性需确认) */}
      <Panel variant="sunken" class="p-[16px] flex flex-col gap-[10px]">
        <Show when={busy()}>
          <p class="text-[13px] text-accent">{t("realm.syncing")}</p>
        </Show>
        <Show when={!busy() && plan()}>
          {(p) => (
            <Show
              when={!(p().download.length === 0 && p().remove.length === 0)}
              fallback={<p class="text-[13px] text-accent">{t("realm.planUpToDate")}</p>}
            >
              <div class="flex flex-wrap gap-[8px] text-[12px]">
                <Show when={p().download.length}>
                  <span class="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planDownload", { count: p().download.length })}</span>
                </Show>
                <Show when={p().remove.length}>
                  <span class="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planRemove", { count: p().remove.length })}</span>
                </Show>
                <Show when={p().manual.length}>
                  <span class="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planManual", { count: p().manual.length })}</span>
                </Show>
              </div>
              <Show when={p().remove.length > 0}>
                <div class="flex items-center justify-between text-[13px] text-fg mt-[4px]">
                  <div class="flex flex-col gap-[2px] min-w-0 pr-[12px]">
                    <span>{t("realm.removeExtras")}</span>
                    <span class="text-[11px] text-muted">{t("realm.removeExtrasHint")}</span>
                  </div>
                  <Toggle checked={removeExtras()} onChange={setRemoveExtras} disabled={busy()} />
                </div>
                <Button variant="primary" class="self-start mt-[4px]" disabled={busy()} onClick={() => void runSync(removeExtras())}>
                  {t("realm.applyChanges")}
                </Button>
              </Show>
              <Show when={p().manual.length > 0}>
                <div class="text-[12px] text-muted mt-[4px]">
                  <div class="mb-[4px]">{t("realm.manualList")}</div>
                  <ul class="list-disc pl-[18px] flex flex-col gap-[2px]">
                    <For each={p().manual}>{(f) => <li class="break-all text-faint">{f.path.replace(/^mods\//, "")}</li>}</For>
                  </ul>
                </div>
              </Show>
            </Show>
          )}
        </Show>
        <Show when={progress()}>
          {(pr) => (
            <div class="h-[6px] w-full bg-window shadow-input rounded-none overflow-hidden">
              <div class="h-full bg-accent transition-[width] duration-150 ease-app" style={{ width: `${pr().total > 0 ? Math.round((pr().current / pr().total) * 100) : 8}%` }} />
            </div>
          )}
        </Show>
        <Show when={canPush()}>
          <div class="pt-[10px] mt-[2px] border-t border-titlebar flex items-center justify-between gap-[10px] flex-wrap">
            <span class="text-[12px] text-muted leading-[1.6] min-w-0">{t("realm.pushHint")}</span>
            <Button variant="ghost" disabled={busy()} onClick={() => void pushManifest()}>
              {busy() ? t("realm.pushing") : t("realm.pushAction")}
            </Button>
          </div>
        </Show>
      </Panel>

      {/* 邀请好友(owner/admin · 社交开启时) */}
      <Show when={canPush() && socialEnabled() && isKobeSignedIn()}>
        <Panel variant="sunken" class="p-[16px]">
          <Heading size="sub" as="h3" class="m-0 text-[14px] mb-[4px]">
            {t("realm.inviteTitle")}
          </Heading>
          <p class="text-[12px] text-muted leading-[1.6] mb-[10px]">{t("realm.inviteHint")}</p>
          <Show when={!friends.loading} fallback={<div class="flex justify-center p-[8px]"><Spinner /></div>}>
            <Show
              when={(friends() ?? []).length > 0}
              fallback={<p class="text-[12px] text-faint">{t("realm.inviteNoFriends")}</p>}
            >
              <div class="flex flex-col gap-[6px]">
                <For each={sortedFriends()}>
                  {(f) => (
                    <Panel variant="raised" class="flex items-center gap-[10px] px-[12px] py-[7px]">
                      <span
                        class={`w-[6px] h-[6px] shrink-0 ${f.online ? "bg-accent" : "bg-faint"}`}
                        aria-hidden="true"
                        title={f.online ? t("friend.online") : t("friend.offline")}
                      />
                      <div class="flex flex-col min-w-0 flex-1">
                        <span class="text-[13px] text-fg truncate">{f.username || f.id.slice(0, 8)}</span>
                        <span class="text-[11px] text-faint truncate">
                          {f.online
                            ? f.activity
                              ? t("friend.playing", { name: f.activity })
                              : t("friend.idle")
                            : t("friend.offline")}
                        </span>
                      </div>
                      <Show
                        when={!memberIds().has(f.id)}
                        fallback={<span class="text-[12px] text-faint">{t("realm.invited")}</span>}
                      >
                        <button
                          class="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy()}
                          onClick={() => void invite(f.id)}
                        >
                          {t("realm.invite")}
                        </button>
                      </Show>
                    </Panel>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </Panel>
      </Show>

      {/* 成员 */}
      <Panel variant="sunken" class="p-[16px]">
        <Heading size="sub" as="h3" class="m-0 text-[14px] mb-[10px]">
          {t("realm.members")}
        </Heading>
        <Show when={!members.loading} fallback={<div class="flex justify-center p-[12px]"><Spinner /></div>}>
          <div class="flex flex-col gap-[8px]">
            <For each={members() ?? []}>
              {(m: RealmMember) => (
                <Panel variant="raised" class="flex items-center gap-[10px] px-[12px] py-[8px]">
                  <div class="flex flex-col min-w-0 flex-1">
                    <span class="text-[13px] text-fg truncate">
                      {m.username || m.user_id.slice(0, 8)}
                      <Show when={m.user_id === myId()}>
                        <span class="text-muted"> {t("realm.you")}</span>
                      </Show>
                    </span>
                    <span class="text-[11px] text-faint">
                      {m.synced_version > 0 ? t("realm.syncedTo", { version: m.synced_version }) : t("realm.notSynced")}
                    </span>
                  </div>
                  <Show when={m.user_id !== myId() && friendIds().has(m.user_id)}>
                    <Tag>{t("realm.friendTag")}</Tag>
                  </Show>
                  <Tag>{roleLabel(m.role)}</Tag>
                  <Show when={isOwner() && m.role !== "owner"}>
                    <div class="flex items-center gap-[6px] shrink-0">
                      <button
                        class="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                        disabled={busy()}
                        onClick={() => void setRole(m.user_id, m.role === "member" ? "admin" : "member")}
                      >
                        {m.role === "member" ? t("realm.promote") : t("realm.demote")}
                      </button>
                      <button
                        class="text-[12px] text-danger-text hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                        disabled={busy()}
                        onClick={() => void removeMember(m.user_id)}
                      >
                        {t("realm.removeMember")}
                      </button>
                    </div>
                  </Show>
                </Panel>
              )}
            </For>
          </div>
        </Show>
        <div class="mt-[12px] pt-[12px] border-t border-titlebar flex justify-end">
          <Button variant="danger" disabled={busy()} onClick={() => setConfirmKind(isOwner() ? "disband" : "leave")}>
            {isOwner() ? t("realm.disband") : t("realm.leave")}
          </Button>
        </div>
      </Panel>

      <Dialog open={confirmKind() !== null} onClose={() => setConfirmKind(null)} label={confirmKind() === "disband" ? t("realm.disband") : t("realm.leave")}>
        <div class="p-[20px] flex flex-col gap-[16px]">
          <p class="text-[14px] text-fg leading-[1.6]">
            {confirmKind() === "disband"
              ? t("realm.confirmDisband", { name: summary()?.name ?? props.instance.name })
              : t("realm.confirmLeave", { name: summary()?.name ?? props.instance.name })}
          </p>
          <div class="flex justify-end gap-[8px]">
            <Button variant="ghost" onClick={() => setConfirmKind(null)}>
              {t("realm.cancel")}
            </Button>
            <Button variant="danger" disabled={busy()} onClick={() => void (confirmKind() === "disband" ? doDisband() : doLeave())}>
              {confirmKind() === "disband" ? t("realm.disband") : t("realm.leave")}
            </Button>
          </div>
        </div>
      </Dialog>

      {/* 未登录兜底(理论上能进到这里说明实例有 realm 绑定但会话过期) */}
      <Show when={!isKobeSignedIn()}>
        <p class="text-[12px] text-muted">
          {t("realm.needLogin")}{" "}
          <button class="text-accent hover:underline bg-transparent border-none cursor-pointer p-0" onClick={() => setCurrentPage("settings")}>
            {t("realm.goLogin")}
          </button>
        </p>
      </Show>
    </div>
  );
};
