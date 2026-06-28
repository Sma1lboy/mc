import {
  Component,
  createEffect,
  createResource,
  createSignal,
  For,
  onCleanup,
  onMount,
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
import {
  activeRoot,
  refreshInstances,
  kobeUser,
  isKobeSignedIn,
  setCurrentPage,
  socialEnabled,
  friends,
  refreshFriends,
} from "../store";
import { avatarTone, avatarInitial } from "../util/avatar";
import { t } from "../i18n";
import type { InstanceSummary, RealmMember, SyncReport, LobbyStatus } from "../ipc/bindings";

/** 折叠区标题左侧的 caret(展开时旋转 90°)。 */
const Caret: Component<{ open: boolean }> = (props) => (
  <svg
    class="w-[10px] h-[10px] shrink-0 text-muted transition-transform duration-150"
    classList={{ "rotate-90": props.open }}
    viewBox="0 0 12 12"
    fill="none"
    aria-hidden="true"
  >
    <path d="M4 2.5 8 6 4 9.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
  </svg>
);

/** 成员 / 邀请头像方块瓦片(含可选在线 pip)。 */
const PersonTile: Component<{ name: string; online?: boolean; pip?: boolean }> = (props) => (
  <span
    class="relative w-[26px] h-[26px] shrink-0 grid place-items-center shadow-raised font-display text-[12px] text-[#1a1b12]"
    classList={{ "grayscale brightness-75": props.pip ? !props.online : false }}
    style={{ "background-color": avatarTone(props.name) }}
    aria-hidden="true"
  >
    {avatarInitial(props.name)}
    <Show when={props.pip}>
      <span
        class="absolute -right-[2px] -bottom-[2px] w-[8px] h-[8px] shadow-[0_0_0_2px_var(--color-panel)]"
        classList={{ "bg-accent": props.online, "bg-faint": !props.online }}
      />
    </Show>
  </span>
);

/** role 字符串 → 本地化标签。 */
function roleLabel(role: string): string {
  return role === "owner"
    ? t("realm.roleOwner")
    : role === "admin"
      ? t("realm.roleAdmin")
      : t("realm.roleMember");
}

/**
 * LobbyBlock —— 领域里的「联机」块:一键开启 / 断开一个 EasyTier 虚拟局域网会话,运行时
 * 轮询状态展示本机虚拟 IP、在线对端与各自的「直连 / 中继 + 延迟」。开启需要管理员 / root
 * 授权(建 TUN);后端按平台提权拉起 easytier-core。EasyTier 未安装时后端返回清晰错误。
 */
const LobbyBlock: Component<{ realmId: string }> = (props) => {
  const [mode, setMode] = createSignal("p2p");
  const [status, setStatus] = createSignal<LobbyStatus | null>(null);
  const [busy, setBusy] = createSignal(false);
  // 免密一键(一次性提权):就绪后开启联机不再弹管理员授权。
  const [privReady, setPrivReady] = createSignal(false);
  const [setupBusy, setSetupBusy] = createSignal(false);
  let timer: number | undefined;

  const running = () => status()?.running ?? false;
  const peers = () => status()?.peers ?? [];

  // creds 仅用于判断是否提供「我们的中继」线路(成员可见,含 secret 故不展示)。
  const [creds] = createResource(
    () => props.realmId,
    (id) => api.realmLobby(id).catch(() => null),
  );
  const hasHosted = () => (creds()?.nodes ?? []).some((n) => n.kind === "hosted");
  const modeOptions = () => [
    { value: "p2p", label: t("lobby.modeP2p") },
    ...(hasHosted() ? [{ value: "hosted", label: t("lobby.modeHosted") }] : []),
  ];

  async function poll() {
    try {
      setStatus(await api.lobbyStatus());
    } catch {
      /* 轮询偶发失败忽略,下一拍再试 */
    }
  }
  function stopPolling() {
    if (timer !== undefined) {
      clearInterval(timer);
      timer = undefined;
    }
  }
  function startPolling() {
    stopPolling();
    timer = window.setInterval(() => void poll(), 3000);
  }

  // 进面板先探一次:可能已有会话在跑(切换实例 / 重开面板),据此恢复轮询。
  onMount(() => {
    void poll().then(() => {
      if (running()) startPolling();
    });
    void api
      .lobbyPrivilegedReady()
      .then(setPrivReady)
      .catch(() => setPrivReady(false));
  });
  onCleanup(stopPolling);

  async function setupPrivileged() {
    if (setupBusy()) return;
    setSetupBusy(true);
    try {
      await api.lobbySetupPrivileged();
      setPrivReady(await api.lobbyPrivilegedReady());
    } catch (e) {
      toast({ type: "error", message: t("lobby.setupError", { err: String(e) }) });
    } finally {
      setSetupBusy(false);
    }
  }

  async function start() {
    if (busy()) return;
    setBusy(true);
    try {
      await api.lobbyStart(props.realmId, mode());
      await poll();
      startPolling();
    } catch (e) {
      toast({ type: "error", message: t("lobby.startError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    if (busy()) return;
    setBusy(true);
    stopPolling();
    try {
      await api.lobbyStop();
      setStatus({ running: false, virtual_ip: null, peers: [] });
    } catch (e) {
      toast({ type: "error", message: t("lobby.stopError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel variant="sunken" class="p-[16px] flex flex-col gap-[10px]">
      <div class="flex items-center justify-between gap-[10px] flex-wrap">
        <div class="flex items-center gap-[8px] min-w-0">
          <span class="text-[12px] text-sub font-display tracking-[0.5px]">{t("lobby.title")}</span>
          <span class="text-[11px]" classList={{ "text-accent": running(), "text-faint": !running() }}>
            {running() ? t("lobby.statusOn") : t("lobby.statusOff")}
          </span>
        </div>
        <div class="flex items-center gap-[8px] shrink-0">
          <Show when={!running()}>
            <Show
              when={!privReady()}
              fallback={<span class="text-[11px] text-faint">{t("lobby.privilegedReady")}</span>}
            >
              <Button variant="ghost" disabled={setupBusy()} onClick={() => void setupPrivileged()}>
                {setupBusy() ? t("lobby.settingUp") : t("lobby.setupPrivileged")}
              </Button>
            </Show>
            <Select value={mode()} onChange={setMode} options={modeOptions()} />
          </Show>
          <Button
            variant={running() ? "ghost" : "primary"}
            disabled={busy()}
            onClick={() => void (running() ? stop() : start())}
          >
            {busy()
              ? running()
                ? t("lobby.stopping")
                : t("lobby.starting")
              : running()
                ? t("lobby.stop")
                : t("lobby.start")}
          </Button>
        </div>
      </div>

      <Show when={running()}>
        <div class="flex items-center gap-[10px] flex-wrap text-[12px]">
          <Show when={status()?.virtual_ip}>
            <span class="bg-window shadow-input px-[8px] py-[3px] font-mono tabular-nums">
              {t("lobby.virtualIp")} · {status()!.virtual_ip}
            </span>
          </Show>
          <span class="text-muted tabular-nums">{t("lobby.peerCount", { n: peers().length })}</span>
        </div>
        <Show when={peers().length > 0} fallback={<p class="text-[12px] text-faint">{t("lobby.noPeers")}</p>}>
          <div class="flex flex-col gap-[2px]">
            <For each={peers()}>
              {(p) => {
                const direct = () => p.cost !== "relay";
                return (
                  <div class="flex items-center gap-[8px] px-[6px] py-[4px] text-[12px]">
                    <span class="text-fg truncate flex-1">{p.hostname}</span>
                    <span
                      classList={{ "text-accent": direct() }}
                      style={direct() ? undefined : { color: "#c9a06a" }}
                    >
                      {direct() ? t("lobby.costDirect") : t("lobby.costRelay")}
                    </span>
                    <Show when={p.lat_ms != null}>
                      <span class="text-faint tabular-nums">{t("lobby.latency", { ms: p.lat_ms ?? 0 })}</span>
                    </Show>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </Show>

      <p class="text-[11px] text-muted leading-[1.6]">{t("lobby.hint")}</p>
      <Show when={!running() && !privReady()}>
        <p class="text-[11px] text-faint leading-[1.6]">{t("lobby.setupHint")}</p>
      </Show>
    </Panel>
  );
};

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

  // 好友列表来自 store(单一真相 + 连续 30s 轮询),用于「邀请好友」与成员的好友标记。
  // 面板打开时主动拉一次,保证 store 缓存新鲜(后续在线状态/活动由 store 轮询维护)。
  onMount(() => void refreshFriends());
  const memberIds = () => new Set((members() ?? []).map((m) => m.user_id));
  const friendIds = () => new Set((friends() ?? []).map((f) => f.id));
  // 成员 → 好友映射(用于在成员行展示在线/活动);非好友成员取不到则无 pip。
  const friendById = () => new Map((friends() ?? []).map((f) => [f.id, f] as const));
  // 折叠头里的「在线 N」:统计同时是好友且在线的成员数。
  const onlineMemberCount = () => (members() ?? []).filter((m) => friendById().get(m.user_id)?.online).length;

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

  // 成员可折叠(默认展开);邀请不折叠,直接一个搜索框,输入才出名字。
  const [membersOpen, setMembersOpen] = createSignal(true);
  const [inviteQuery, setInviteQuery] = createSignal("");
  // 邀请:在自己的好友里按用户名过滤(空输入不出名单),在线优先。
  const inviteMatches = () => {
    const q = inviteQuery().trim().toLowerCase();
    if (!q) return [];
    return sortedFriends().filter((f) => (f.username || f.id).toLowerCase().includes(q));
  };

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
      {/* 头部:领域身份(头像 + 名称 + 角色 + 版本·成员数副行)+ 可复制加入码 chip */}
      {(() => {
        const realmName = () => summary()?.name ?? props.instance.realm!.name ?? t("realm.title");
        return (
          <Panel variant="sunken" class="p-[16px] flex items-center justify-between gap-[12px] flex-wrap">
            <div class="flex items-center gap-[10px] min-w-0">
              <PersonTile name={realmName()} />
              <div class="flex flex-col min-w-0 gap-[2px]">
                <div class="flex items-center gap-[8px] min-w-0">
                  <Heading size="sub" as="h3" class="m-0 text-[14px] truncate">
                    {realmName()}
                  </Heading>
                  <Tag>{roleLabel(role())}</Tag>
                </div>
                <Show when={summary()}>
                  <span class="text-[11px] text-muted tabular-nums">
                    {t("realm.manifestVersion", { version: summary()!.manifest_version })}
                    {" · "}
                    {t("realm.memberCount", { n: (members() ?? []).length })}
                  </span>
                </Show>
              </div>
            </div>
            <Show when={code()}>
              <button
                type="button"
                class="inline-flex items-center gap-[8px] font-mono text-[14px] text-accent tracking-[0.16em] tabular-nums bg-window shadow-input px-[10px] py-[5px] cursor-pointer hover:brightness-110 [-webkit-app-region:no-drag]"
                title={t("realm.copyCode")}
                onClick={() => void copyCode()}
              >
                <span>{code()}</span>
                <svg class="w-[13px] h-[13px] shrink-0 opacity-70" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                  <rect x="9" y="9" width="11" height="11" rx="1.5" stroke="currentColor" stroke-width="1.8" />
                  <path d="M5 15V5.5A1.5 1.5 0 0 1 6.5 4H15" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" />
                </svg>
              </button>
            </Show>
          </Panel>
        );
      })()}

      {/* 联机大厅(EasyTier 虚拟局域网):社交开启时出现 */}
      <Show when={socialEnabled()}>
        <LobbyBlock realmId={rid()} />
      </Show>

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
              {/* 一句话状态:领域有更新,点下面同步。 */}
              <p class="text-[13px] text-strong">{t("realm.syncPending")}</p>
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
              {/* 移除开关:仅当有「领域之外的 mod」可移除时出现。 */}
              <Show when={p().remove.length > 0}>
                <div class="flex items-center justify-between text-[13px] text-fg mt-[4px]">
                  <div class="flex flex-col gap-[2px] min-w-0 pr-[12px]">
                    <span>{t("realm.removeExtras")}</span>
                    <span class="text-[11px] text-muted">{t("realm.removeExtrasHint")}</span>
                  </div>
                  <Toggle checked={removeExtras()} onChange={setRemoveExtras} disabled={busy()} />
                </div>
              </Show>
              {/* 同步按钮:只要有待同步项就显示(不再只在有移除时出现)。 */}
              <Button variant="primary" class="self-start mt-[4px]" disabled={busy()} onClick={() => void runSync(removeExtras())}>
                {t("realm.applyChanges")}
              </Button>
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

      {/* 成员(可折叠;上移到邀请之前) */}
      <Panel variant="sunken" class="p-0 overflow-hidden">
        <button
          type="button"
          class="w-full flex items-center gap-[8px] px-[16px] py-[12px] bg-transparent border-none cursor-pointer text-left hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app"
          onClick={() => setMembersOpen((o) => !o)}
        >
          <Caret open={membersOpen()} />
          <span class="text-[12px] text-sub font-display tracking-[0.5px]">{t("realm.members")}</span>
          <span class="text-[11px] text-faint tabular-nums">{(members() ?? []).length}</span>
          <Show when={onlineMemberCount() > 0}>
            <span class="text-[11px] text-accent tabular-nums">{t("friend.onlineCount", { n: onlineMemberCount() })}</span>
          </Show>
          {/* 折叠时:在标题行右侧叠展成员头像(peek)。 */}
          <Show when={!membersOpen()}>
            <span class="flex-1" />
            <span class="flex items-center -space-x-[6px] pr-[2px]">
              <For each={(members() ?? []).slice(0, 6)}>
                {(m) => {
                  const nm = m.username || m.user_id.slice(0, 8);
                  return (
                    <span
                      class="w-[20px] h-[20px] grid place-items-center shadow-raised font-display text-[10px] text-[#1a1b12] ring-2 ring-panel"
                      style={{ "background-color": avatarTone(nm) }}
                      aria-hidden="true"
                    >
                      {avatarInitial(nm)}
                    </span>
                  );
                }}
              </For>
              <Show when={(members() ?? []).length > 6}>
                <span class="text-[11px] text-faint pl-[10px] tabular-nums">+{(members() ?? []).length - 6}</span>
              </Show>
            </span>
          </Show>
        </button>
        <Show when={membersOpen()}>
          <div class="px-[8px] pb-[10px]">
            <Show when={!members.loading} fallback={<div class="flex justify-center p-[12px]"><Spinner /></div>}>
              <For each={members() ?? []}>
                {(m: RealmMember) => {
                  const nm = m.username || m.user_id.slice(0, 8);
                  // 同时是好友的成员:借好友列表(store 轮询维护在线/活动)展示在线状态。
                  const friend = () => friendById().get(m.user_id);
                  const isFriend = () => friend() !== undefined;
                  return (
                    <div class="flex items-center gap-[10px] px-[8px] py-[6px] group hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app">
                      <PersonTile name={nm} online={friend()?.online} pip={isFriend()} />
                      <div class="flex flex-col min-w-0 flex-1">
                        <span class="text-[13px] text-fg truncate">
                          {nm}
                          <Show when={m.user_id === myId()}>
                            <span class="text-muted"> {t("realm.you")}</span>
                          </Show>
                        </span>
                        <Show when={friend()?.online ? friend() : undefined}>
                          {(f) => (
                            <span class="text-[11px] text-accent truncate">
                              {f().activity ? t("friend.playing", { name: f().activity ?? "" }) : t("friend.idle")}
                            </span>
                          )}
                        </Show>
                        <span class="text-[11px] text-faint truncate">
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
                    </div>
                  );
                }}
              </For>
            </Show>
          </div>
        </Show>
      </Panel>

      {/* 邀请好友(owner/admin · 社交开启时):不折叠,直接一个搜索框,输入才出名字 */}
      <Show when={canPush() && socialEnabled() && isKobeSignedIn()}>
        <Panel variant="sunken" class="p-[16px] flex flex-col gap-[8px]">
          <span class="text-[12px] text-sub font-display tracking-[0.5px]">{t("realm.inviteTitle")}</span>
          <input
            class="h-[32px] px-[10px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            type="text"
            placeholder={t("realm.inviteSearchPlaceholder")}
            value={inviteQuery()}
            onInput={(e) => setInviteQuery(e.currentTarget.value)}
          />
          <Show when={inviteQuery().trim().length > 0}>
            <Show
              when={inviteMatches().length > 0}
              fallback={<p class="text-[12px] text-faint px-[2px]">{t("friend.noResults")}</p>}
            >
              <div class="flex flex-col gap-[2px] max-h-[200px] overflow-y-auto">
                <For each={inviteMatches()}>
                  {(f) => {
                    const nm = f.username || f.id.slice(0, 8);
                    return (
                      <div class="flex items-center gap-[10px] px-[8px] py-[6px]" classList={{ "opacity-70": !f.online }}>
                        <PersonTile name={nm} online={f.online} pip />
                        <div class="flex flex-col min-w-0 flex-1">
                          <span class="text-[13px] text-fg truncate">{nm}</span>
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
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>
          </Show>
        </Panel>
      </Show>

      {/* 退出 / 解散 */}
      <div class="flex justify-end">
        <Button variant="danger" disabled={busy()} onClick={() => setConfirmKind(isOwner() ? "disband" : "leave")}>
          {isOwner() ? t("realm.disband") : t("realm.leave")}
        </Button>
      </div>

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
