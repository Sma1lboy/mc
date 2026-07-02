import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
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
  setCurrentPage,
  refreshFriends,
  playInstance,
  useAppStore,
} from "../store";
import { useAsync } from "../util/useAsync";
import { avatarTone, avatarInitial } from "../util/avatar";
import { t, useLang } from "../i18n";
import type { InstanceSummary, RealmMember, SyncReport, LobbyStatus, RealmHost } from "../ipc/bindings";

/** 折叠区标题左侧的 caret(展开时旋转 90°)。 */
function Caret({ open }: { open: boolean }) {
  return (
    <svg
      className={clsx("w-[10px] h-[10px] shrink-0 text-muted transition-transform duration-150", { "rotate-90": open })}
      viewBox="0 0 12 12"
      fill="none"
      aria-hidden="true"
    >
      <path d="M4 2.5 8 6 4 9.5" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/** 成员 / 邀请头像方块瓦片(含可选在线 pip)。 */
function PersonTile({ name, online, pip }: { name: string; online?: boolean; pip?: boolean }) {
  return (
    <span
      className={clsx(
        "relative w-[26px] h-[26px] shrink-0 grid place-items-center shadow-raised font-display text-[12px] text-[#1a1b12]",
        { "grayscale brightness-75": pip ? !online : false },
      )}
      style={{ backgroundColor: avatarTone(name) }}
      aria-hidden="true"
    >
      {avatarInitial(name)}
      {pip && (
        <span
          className={clsx(
            "absolute -right-[2px] -bottom-[2px] w-[8px] h-[8px] shadow-[0_0_0_2px_var(--color-panel)]",
            online ? "bg-accent" : "bg-faint",
          )}
        />
      )}
    </span>
  );
}

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
function LobbyBlock({ realmId, instanceId }: { realmId: string; instanceId: string }) {
  const [mode, setMode] = useState("p2p");
  const [status, setStatus] = useState<LobbyStatus | null>(null);
  const [busy, setBusy] = useState(false);
  // 免密一键(一次性提权):就绪后开启联机不再弹管理员授权。
  const [privReady, setPrivReady] = useState(false);
  const [setupBusy, setSetupBusy] = useState(false);
  // 联机大厅 P3:本机探到的 MC「对局域网开放」端口(我在主持);server 上当前的 host(谁在主持)。
  const [lanPort, setLanPort] = useState<number | null>(null);
  const [host, setHost] = useState<RealmHost | null>(null);

  const instLaunching = useAppStore((s) => s.launchingIds.has(instanceId));
  const instRunning = useAppStore((s) => s.runningIds.has(instanceId));

  const timerRef = useRef<number | undefined>(undefined);
  const hostTimerRef = useRef<number | undefined>(undefined);
  const lastHeartbeatRef = useRef(0);
  // 轮询/主持回调在定时器里跑,读「当时」的 status 需走 ref(旧闭包会拿到过期 status)。
  const statusRef = useRef<LobbyStatus | null>(null);
  const applyStatus = (s: LobbyStatus | null) => {
    statusRef.current = s;
    setStatus(s);
  };

  const running = status?.running ?? false;
  const peers = status?.peers ?? [];
  const virtualIp = status?.virtual_ip ?? null;
  // 「我就是 host」:本机正广播 LAN-open(探到端口),或 server 上的 host 地址正落在我的虚拟 IP 上。
  const amHost = (() => {
    if (lanPort != null) return true;
    const addr = host?.address;
    return !!(virtualIp && addr && addr.startsWith(`${virtualIp}:`));
  })();
  // 成员侧:存在新鲜且非我的 host 地址 → 可一键加入。
  const joinableHost = (() => {
    const addr = host?.address;
    return addr && !amHost ? addr : null;
  })();
  const joinDisabled = instLaunching || instRunning;

  // creds 仅用于判断是否提供「我们的中继」线路(成员可见,含 secret 故不展示)。
  const { data: creds } = useAsync(() => api.realmLobby(realmId).catch(() => null), [realmId]);
  const hasHosted = (creds?.nodes ?? []).some((n) => n.kind === "hosted");
  const modeOptions = [
    { value: "p2p", label: t("lobby.modeP2p") },
    ...(hasHosted ? [{ value: "hosted", label: t("lobby.modeHosted") }] : []),
  ];

  async function poll() {
    try {
      applyStatus(await api.lobbyStatus());
    } catch {
      /* 轮询偶发失败忽略,下一拍再试 */
    }
  }
  function stopPolling() {
    if (timerRef.current !== undefined) {
      clearInterval(timerRef.current);
      timerRef.current = undefined;
    }
  }
  function startPolling() {
    stopPolling();
    timerRef.current = window.setInterval(() => void poll(), 3000);
  }

  // 联机大厅 P3 —— 一拍 host/member 闭环(联机运行时每 ~5s 跑一次):
  //  · host 侧:探本机 MC「对局域网开放」端口;探到且有虚拟 IP → 发布 `虚拟IP:端口`,并每
  //    ~30s 续约作心跳(server 90s 过期),让成员能看到我在主持。
  //  · member 侧:拉 server 上当前(新鲜的)host;非我自己 → 显示「加入游戏」。
  async function hostTick() {
    const st = statusRef.current;
    if (!st?.running) return;
    const ip = st.virtual_ip ?? null;
    // host 侧探测(只有拿到虚拟 IP 才有意义,地址要靠它拼)。
    if (ip) {
      try {
        const port = await api.detectLanWorld();
        setLanPort(port ?? null);
        if (port != null) {
          const now = Date.now();
          if (now - lastHeartbeatRef.current > 30_000) {
            await api.realmSetHost(realmId, `${ip}:${port}`);
            lastHeartbeatRef.current = now;
          }
        }
      } catch {
        /* 探测/发布偶发失败忽略,下一拍再试 */
      }
    }
    // member 侧:谁在主持。
    try {
      setHost(await api.realmGetHost(realmId));
    } catch {
      /* 忽略 */
    }
  }
  function stopHostLoop() {
    if (hostTimerRef.current !== undefined) {
      clearInterval(hostTimerRef.current);
      hostTimerRef.current = undefined;
    }
    setLanPort(null);
    setHost(null);
    lastHeartbeatRef.current = 0;
  }
  function startHostLoop() {
    if (hostTimerRef.current !== undefined) return;
    void hostTick();
    hostTimerRef.current = window.setInterval(() => void hostTick(), 5000);
  }

  // 进面板先探一次:可能已有会话在跑(切换实例 / 重开面板),据此恢复轮询。
  // 卸载即停轮询/主持,并 best-effort 告诉 server 我不再主持。
  useEffect(() => {
    void poll().then(() => {
      if (statusRef.current?.running) {
        startPolling();
        startHostLoop();
      }
    });
    void api
      .lobbyPrivilegedReady()
      .then(setPrivReady)
      .catch(() => setPrivReady(false));
    return () => {
      stopPolling();
      stopHostLoop();
      void api.realmClearHost(realmId).catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function setupPrivileged() {
    if (setupBusy) return;
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
    if (busy) return;
    setBusy(true);
    try {
      await api.lobbyStart(realmId, mode);
      await poll();
      startPolling();
      startHostLoop();
    } catch (e) {
      toast({ type: "error", message: t("lobby.startError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  async function stop() {
    if (busy) return;
    setBusy(true);
    stopPolling();
    stopHostLoop();
    try {
      // 断开前先停止主持,成员立刻不再看到我(否则要等 server 90s 过期)。
      await api.realmClearHost(realmId).catch(() => {});
      await api.lobbyStop();
      applyStatus({ running: false, virtual_ip: null, peers: [] });
    } catch (e) {
      toast({ type: "error", message: t("lobby.stopError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  async function join() {
    const addr = joinableHost;
    if (!addr || joinDisabled) return;
    try {
      await playInstance(instanceId, addr);
    } catch (e) {
      toast({ type: "error", message: t("lobby.joinError", { err: String(e) }) });
    }
  }

  return (
    <Panel variant="sunken" className="p-[16px] flex flex-col gap-[10px]">
      <div className="flex items-center justify-between gap-[10px] flex-wrap">
        <div className="flex items-center gap-[8px] min-w-0">
          <span className="text-[12px] text-sub font-display tracking-[0.5px]">{t("lobby.title")}</span>
          <span className={clsx("text-[11px]", { "text-accent": running, "text-faint": !running })}>
            {running ? t("lobby.statusOn") : t("lobby.statusOff")}
          </span>
        </div>
        <div className="flex items-center gap-[8px] shrink-0">
          {!running && (
            <>
              {!privReady ? (
                <Button variant="ghost" disabled={setupBusy} onClick={() => void setupPrivileged()}>
                  {setupBusy ? t("lobby.settingUp") : t("lobby.setupPrivileged")}
                </Button>
              ) : (
                <span className="text-[11px] text-faint">{t("lobby.privilegedReady")}</span>
              )}
              <Select value={mode} onChange={setMode} options={modeOptions} />
            </>
          )}
          <Button variant={running ? "ghost" : "primary"} disabled={busy} onClick={() => void (running ? stop() : start())}>
            {busy
              ? running
                ? t("lobby.stopping")
                : t("lobby.starting")
              : running
                ? t("lobby.stop")
                : t("lobby.start")}
          </Button>
        </div>
      </div>

      {running && (
        <>
          <div className="flex items-center gap-[10px] flex-wrap text-[12px]">
            {status?.virtual_ip && (
              <span className="bg-window shadow-input px-[8px] py-[3px] font-mono tabular-nums">
                {t("lobby.virtualIp")} · {status.virtual_ip}
              </span>
            )}
            <span className="text-muted tabular-nums">{t("lobby.peerCount", { n: peers.length })}</span>
          </div>
          {peers.length > 0 ? (
            <div className="flex flex-col gap-[2px]">
              {peers.map((p) => {
                const direct = p.cost !== "relay";
                return (
                  <div key={p.hostname} className="flex items-center gap-[8px] px-[6px] py-[4px] text-[12px]">
                    <span className="text-fg truncate flex-1">{p.hostname}</span>
                    <span className={clsx({ "text-accent": direct })} style={direct ? undefined : { color: "#c9a06a" }}>
                      {direct ? t("lobby.costDirect") : t("lobby.costRelay")}
                    </span>
                    {p.lat_ms != null && (
                      <span className="text-faint tabular-nums">{t("lobby.latency", { ms: p.lat_ms ?? 0 })}</span>
                    )}
                  </div>
                );
              })}
            </div>
          ) : (
            <p className="text-[12px] text-faint">{t("lobby.noPeers")}</p>
          )}

          {/* 联机大厅 P3:开世界 / 加入游戏闭环 */}
          <div className="flex items-center gap-[10px] flex-wrap pt-[2px]">
            {joinableHost ? (
              <>
                <span className="text-[12px] text-muted truncate min-w-0">
                  {t("lobby.hostedBy", { name: host?.host_username ?? t("lobby.someone") })}
                </span>
                <Button variant="primary" disabled={joinDisabled} onClick={() => void join()}>
                  {instRunning || instLaunching ? t("lobby.joining") : t("lobby.join")}
                </Button>
              </>
            ) : amHost ? (
              <span className="text-[12px] text-accent">{t("lobby.hostingNow", { port: lanPort ?? 0 })}</span>
            ) : (
              <span className="text-[12px] text-faint leading-[1.6]">{t("lobby.openWorldHint")}</span>
            )}
          </div>
        </>
      )}

      <p className="text-[11px] text-muted leading-[1.6]">{t("lobby.hint")}</p>
      {!running && !privReady && <p className="text-[11px] text-faint leading-[1.6]">{t("lobby.setupHint")}</p>}
    </Panel>
  );
}

/**
 * RealmPanel —— 实例详情里的「领域」段。把领域完全收进 instance 入口:
 * - 非领域实例:一个「分享为领域」入口。
 * - 已加入但未装核心(pending):一个「开始同步(Begin)」按钮 —— 装版本/loader + 下 mods。
 * - 已装核心的领域实例:加入码 / 成员 / 自动检测+同步 / 推送清单(owner·admin)/ 退出·解散。
 */
export function RealmPanel({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  useLang();
  const realm = instance.realm;
  return (
    <div className="px-[28px] py-[14px]">
      {!realm ? (
        <ShareEntry instance={instance} onChanged={onChanged} />
      ) : instance.installed ? (
        <RealmManage instance={instance} onChanged={onChanged} />
      ) : (
        <BeginEntry instance={instance} onChanged={onChanged} />
      )}
    </div>
  );
}

/* ---------- 非领域实例:分享入口 ---------- */

function ShareEntry({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  const [open, setOpen] = useState(false);
  return (
    <Panel variant="sunken" className="p-[16px] flex items-center justify-between gap-[12px] flex-wrap">
      <div className="min-w-0">
        <div className="flex items-center gap-[8px]">
          <Heading size="sub" as="h3" className="m-0 text-[14px]">
            {t("realm.title")}
          </Heading>
        </div>
        <p className="text-[12px] text-muted mt-[4px] leading-[1.6]">{t("realm.shareHint")}</p>
      </div>
      <Button variant="ghost" onClick={() => setOpen(true)}>
        {t("realm.shareAction")}
      </Button>
      <ShareDialog instance={instance} open={open} onClose={() => setOpen(false)} onShared={onChanged} />
    </Panel>
  );
}

function ShareDialog({
  instance,
  open,
  onClose,
  onShared,
}: {
  instance: InstanceSummary;
  open: boolean;
  onClose: () => void;
  onShared?: () => void;
}) {
  const [name, setName] = useState(instance.name);
  const [expiry, setExpiry] = useState("0");
  const [busy, setBusy] = useState(false);
  const kobeSignedIn = useAppStore((s) => s.kobeUser !== null);

  async function submit() {
    if (busy || !name.trim()) return;
    if (!kobeSignedIn) {
      toast({ type: "error", message: t("realm.needLogin") });
      return;
    }
    setBusy(true);
    try {
      const secs = parseInt(expiry, 10) || 0;
      const r = await api.realmCreate(
        activeRoot(),
        instance.id,
        name.trim(),
        instance.mc_version,
        instance.loader ?? "vanilla",
        instance.loader_version ?? null,
        secs > 0 ? secs : null,
      );
      toast({ type: "success", message: t("realm.createdToast", { name: r.name }) });
      onClose();
      onShared?.();
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onClose={onClose} label={t("realm.shareTitle")}>
      <div className="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub" as="h2" className="m-0">
          {t("realm.shareTitle")}
        </Heading>
        {!kobeSignedIn && <p className="text-[12px] text-danger-text">{t("realm.needLogin")}</p>}
        <label className="flex flex-col gap-[6px]">
          <span className="text-[12px] text-muted">{t("realm.nameLabel")}</span>
          <input
            className="h-[38px] px-[14px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            type="text"
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
          />
        </label>
        <label className="flex flex-col gap-[6px]">
          <span className="text-[12px] text-muted">{t("realm.expiry")}</span>
          <Select
            value={expiry}
            onChange={setExpiry}
            options={[
              { value: "0", label: t("realm.expiryNever") },
              { value: "86400", label: t("realm.expiry1d") },
              { value: "604800", label: t("realm.expiry7d") },
              { value: "2592000", label: t("realm.expiry30d") },
            ]}
          />
        </label>
        <div className="flex justify-end gap-[8px] mt-[4px]">
          <Button variant="ghost" onClick={onClose}>
            {t("realm.cancel")}
          </Button>
          <Button variant="primary" disabled={busy || !name.trim()} onClick={() => void submit()}>
            {busy ? t("realm.creating") : t("realm.shareAction")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/* ---------- pending:开始同步 ---------- */

function BeginEntry({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  const r = instance.realm!;
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  useEffect(() => onRealmSyncProgress((p) => setProgress({ current: p.current, total: p.total })), []);

  async function begin() {
    if (busy) return;
    setBusy(true);
    setProgress({ current: 0, total: 0 });
    try {
      const report: SyncReport = await api.realmBegin(r.realm_id, activeRoot(), instance.id);
      refreshInstances();
      onChanged?.();
      toast({
        type: report.failed.length ? "error" : "success",
        message: report.failed.length ? t("realm.syncFailed", { count: report.failed.length }) : t("realm.beginDone"),
      });
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  return (
    <Panel variant="sunken" className="p-[20px] flex flex-col gap-[12px]">
      <div className="flex items-center gap-[8px] flex-wrap">
        <Heading size="sub" as="h3" className="m-0 text-[14px]">
          {r.name || t("realm.title")}
        </Heading>
        <Tag>{roleLabel(r.role)}</Tag>
        {r.code && <span className="font-mono text-[12px] text-accent tracking-[0.12em]">{r.code}</span>}
      </div>
      <p className="text-[12px] text-muted leading-[1.6]">{t("realm.beginHint")}</p>
      <Button variant="primary" className="self-start" disabled={busy} onClick={() => void begin()}>
        {busy ? t("realm.syncing") : t("realm.beginAction")}
      </Button>
      {progress && (
        <div className="h-[6px] w-full bg-window shadow-input rounded-none overflow-hidden">
          <div
            className="h-full bg-accent transition-[width] duration-150 ease-app"
            style={{ width: `${progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 8}%` }}
          />
        </div>
      )}
    </Panel>
  );
}

/* ---------- 已装核心的领域实例:管理 ---------- */

function RealmManage({ instance, onChanged }: { instance: InstanceSummary; onChanged?: () => void }) {
  const rid = instance.realm!.realm_id;
  const myId = useAppStore((s) => s.kobeUser?.id);
  // 好友列表来自 store(单一真相 + 连续 30s 轮询),用于「邀请好友」与成员的好友标记。
  const friendsList = useAppStore((s) => s.friends);
  const socialOn = useAppStore((s) => s.socialEnabled);
  const kobeSignedIn = useAppStore((s) => s.kobeUser !== null);

  const { data: summary, refetch: refetchSummary } = useAsync(() => api.realmGet(rid), [rid]);
  const { data: members, loading: membersLoading, refetch: refetchMembers } = useAsync(() => api.realmMembers(rid), [rid]);
  const role = summary?.role ?? instance.realm!.role;
  const isOwner = role === "owner";
  const canPush = role === "owner" || role === "admin";

  // 面板打开时主动拉一次好友,保证 store 缓存新鲜(后续在线状态/活动由 store 轮询维护)。
  useEffect(() => {
    void refreshFriends();
  }, []);
  const memberIds = new Set((members ?? []).map((m) => m.user_id));
  const friendIds = new Set((friendsList ?? []).map((f) => f.id));
  // 成员 → 好友映射(用于在成员行展示在线/活动);非好友成员取不到则无 pip。
  const friendById = new Map((friendsList ?? []).map((f) => [f.id, f] as const));
  // 折叠头里的「在线 N」:统计同时是好友且在线的成员数。
  const onlineMemberCount = (members ?? []).filter((m) => friendById.get(m.user_id)?.online).length;

  // 在线好友优先,其次按用户名排序,便于优先邀请在线好友。
  const sortedFriends = [...(friendsList ?? [])].sort((a, b) => {
    if (!!a.online !== !!b.online) return a.online ? -1 : 1;
    return (a.username || a.id).localeCompare(b.username || b.id);
  });

  const [removeExtras, setRemoveExtras] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  const [confirmKind, setConfirmKind] = useState<"leave" | "disband" | null>(null);

  // 成员可折叠(默认展开);邀请不折叠,直接一个搜索框,输入才出名字。
  const [membersOpen, setMembersOpen] = useState(true);
  const [inviteQuery, setInviteQuery] = useState("");
  // 邀请:在自己的好友里按用户名过滤(空输入不出名单),在线优先。
  const inviteMatches = (() => {
    const q = inviteQuery.trim().toLowerCase();
    if (!q) return [];
    return sortedFriends.filter((f) => (f.username || f.id).toLowerCase().includes(q));
  })();

  // 自动检测差异:随 (领域, 清单版本) 自动重算(summary 未就绪时不打后端)。
  const planKey = summary ? { rid, iid: instance.id, mv: summary.manifest_version } : null;
  const { data: plan, refetch: refetchPlan } = useAsync(
    () => (planKey ? api.realmPlanSync(planKey.rid, activeRoot(), planKey.iid) : Promise.resolve(undefined)),
    [rid, instance.id, summary?.manifest_version],
  );

  useEffect(() => onRealmSyncProgress((p) => setProgress({ current: p.current, total: p.total })), []);

  function fail(e: unknown) {
    toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
  }

  // 纯新增差异自动同步;有移除(破坏性)留给显式确认。去重避免死循环。
  const autoKeyRef = useRef("");
  useEffect(() => {
    if (!plan || busy) return;
    const key = `${rid}:${plan.version}`;
    if (plan.download.length > 0 && plan.remove.length === 0 && autoKeyRef.current !== key) {
      autoKeyRef.current = key;
      void runSync(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan, busy, rid]);

  async function runSync(remove: boolean) {
    if (busy) return;
    setBusy(true);
    setProgress({ current: 0, total: 0 });
    try {
      const report: SyncReport = await api.realmSync(rid, activeRoot(), instance.id, remove);
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
        rid,
        activeRoot(),
        instance.id,
        instance.mc_version,
        instance.loader ?? "vanilla",
        instance.loader_version ?? null,
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
    const c = summary?.code ?? instance.realm!.code;
    if (!c) return;
    try {
      await navigator.clipboard.writeText(c);
      toast({ type: "success", message: t("realm.copied") });
    } catch (e) {
      fail(e);
    }
  }

  async function setRole(uid: string, r: string) {
    setBusy(true);
    try {
      await api.realmSetRole(rid, uid, r);
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
      await api.realmRemoveMember(rid, uid);
      await refetchMembers();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function invite(uid: string) {
    if (busy) return;
    setBusy(true);
    try {
      await api.realmInvite(rid, uid);
      await refetchMembers();
      toast({ type: "success", message: t("realm.inviteDone") });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }

  async function doLeave() {
    if (!myId) return;
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmLeave(rid, myId, activeRoot(), instance.id);
      refreshInstances();
      onChanged?.();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  async function doDisband() {
    setConfirmKind(null);
    setBusy(true);
    try {
      await api.realmDisband(rid, activeRoot(), instance.id);
      refreshInstances();
      onChanged?.();
    } catch (e) {
      fail(e);
      setBusy(false);
    }
  }

  const code = summary?.code ?? instance.realm!.code ?? "";
  const realmName = summary?.name ?? instance.realm!.name ?? t("realm.title");

  return (
    <div className="flex flex-col gap-[12px]">
      {/* 头部:领域身份(头像 + 名称 + 角色 + 版本·成员数副行)+ 可复制加入码 chip */}
      <Panel variant="sunken" className="p-[16px] flex items-center justify-between gap-[12px] flex-wrap">
        <div className="flex items-center gap-[10px] min-w-0">
          <PersonTile name={realmName} />
          <div className="flex flex-col min-w-0 gap-[2px]">
            <div className="flex items-center gap-[8px] min-w-0">
              <Heading size="sub" as="h3" className="m-0 text-[14px] truncate">
                {realmName}
              </Heading>
              <Tag>{roleLabel(role)}</Tag>
            </div>
            {summary && (
              <span className="text-[11px] text-muted tabular-nums">
                {t("realm.manifestVersion", { version: summary.manifest_version })}
                {" · "}
                {t("realm.memberCount", { n: (members ?? []).length })}
              </span>
            )}
          </div>
        </div>
        {code && (
          <button
            type="button"
            className="inline-flex items-center gap-[8px] font-mono text-[14px] text-accent tracking-[0.16em] tabular-nums bg-window shadow-input px-[10px] py-[5px] cursor-pointer hover:brightness-110 [-webkit-app-region:no-drag]"
            title={t("realm.copyCode")}
            onClick={() => void copyCode()}
          >
            <span>{code}</span>
            <svg className="w-[13px] h-[13px] shrink-0 opacity-70" viewBox="0 0 24 24" fill="none" aria-hidden="true">
              <rect x="9" y="9" width="11" height="11" rx="1.5" stroke="currentColor" strokeWidth="1.8" />
              <path d="M5 15V5.5A1.5 1.5 0 0 1 6.5 4H15" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
            </svg>
          </button>
        )}
      </Panel>

      {/* 联机大厅(EasyTier 虚拟局域网):社交开启时出现 */}
      {socialOn && <LobbyBlock realmId={rid} instanceId={instance.id} />}

      {/* 同步状态(自动检测 + 自动同步;破坏性需确认) */}
      <Panel variant="sunken" className="p-[16px] flex flex-col gap-[10px]">
        {busy && <p className="text-[13px] text-accent">{t("realm.syncing")}</p>}
        {!busy && plan && (
          plan.download.length === 0 && plan.remove.length === 0 ? (
            <p className="text-[13px] text-accent">{t("realm.planUpToDate")}</p>
          ) : (
            <>
              {/* 一句话状态:领域有更新,点下面同步。 */}
              <p className="text-[13px] text-strong">{t("realm.syncPending")}</p>
              <div className="flex flex-wrap gap-[8px] text-[12px]">
                {plan.download.length > 0 && (
                  <span className="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planDownload", { count: plan.download.length })}</span>
                )}
                {plan.remove.length > 0 && (
                  <span className="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planRemove", { count: plan.remove.length })}</span>
                )}
                {plan.manual.length > 0 && (
                  <span className="bg-window shadow-input px-[8px] py-[3px]">{t("realm.planManual", { count: plan.manual.length })}</span>
                )}
              </div>
              {/* 移除开关:仅当有「领域之外的 mod」可移除时出现。 */}
              {plan.remove.length > 0 && (
                <div className="flex items-center justify-between text-[13px] text-fg mt-[4px]">
                  <div className="flex flex-col gap-[2px] min-w-0 pr-[12px]">
                    <span>{t("realm.removeExtras")}</span>
                    <span className="text-[11px] text-muted">{t("realm.removeExtrasHint")}</span>
                  </div>
                  <Toggle checked={removeExtras} onChange={setRemoveExtras} disabled={busy} />
                </div>
              )}
              {/* 同步按钮:只要有待同步项就显示(不再只在有移除时出现)。 */}
              <Button variant="primary" className="self-start mt-[4px]" disabled={busy} onClick={() => void runSync(removeExtras)}>
                {t("realm.applyChanges")}
              </Button>
              {plan.manual.length > 0 && (
                <div className="text-[12px] text-muted mt-[4px]">
                  <div className="mb-[4px]">{t("realm.manualList")}</div>
                  <ul className="list-disc pl-[18px] flex flex-col gap-[2px]">
                    {plan.manual.map((f) => (
                      <li key={f.path} className="break-all text-faint">
                        {f.path.replace(/^mods\//, "")}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </>
          )
        )}
        {progress && (
          <div className="h-[6px] w-full bg-window shadow-input rounded-none overflow-hidden">
            <div
              className="h-full bg-accent transition-[width] duration-150 ease-app"
              style={{ width: `${progress.total > 0 ? Math.round((progress.current / progress.total) * 100) : 8}%` }}
            />
          </div>
        )}
        {canPush && (
          <div className="pt-[10px] mt-[2px] border-t border-titlebar flex items-center justify-between gap-[10px] flex-wrap">
            <span className="text-[12px] text-muted leading-[1.6] min-w-0">{t("realm.pushHint")}</span>
            <Button variant="ghost" disabled={busy} onClick={() => void pushManifest()}>
              {busy ? t("realm.pushing") : t("realm.pushAction")}
            </Button>
          </div>
        )}
      </Panel>

      {/* 成员(可折叠;上移到邀请之前) */}
      <Panel variant="sunken" className="p-0 overflow-hidden">
        <button
          type="button"
          className="w-full flex items-center gap-[8px] px-[16px] py-[12px] bg-transparent border-none cursor-pointer text-left hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app"
          onClick={() => setMembersOpen((o) => !o)}
        >
          <Caret open={membersOpen} />
          <span className="text-[12px] text-sub font-display tracking-[0.5px]">{t("realm.members")}</span>
          <span className="text-[11px] text-faint tabular-nums">{(members ?? []).length}</span>
          {onlineMemberCount > 0 && (
            <span className="text-[11px] text-accent tabular-nums">{t("friend.onlineCount", { n: onlineMemberCount })}</span>
          )}
          {/* 折叠时:在标题行右侧叠展成员头像(peek)。 */}
          {!membersOpen && (
            <>
              <span className="flex-1" />
              <span className="flex items-center -space-x-[6px] pr-[2px]">
                {(members ?? []).slice(0, 6).map((m) => {
                  const nm = m.username || m.user_id.slice(0, 8);
                  return (
                    <span
                      key={m.user_id}
                      className="w-[20px] h-[20px] grid place-items-center shadow-raised font-display text-[10px] text-[#1a1b12] ring-2 ring-panel"
                      style={{ backgroundColor: avatarTone(nm) }}
                      aria-hidden="true"
                    >
                      {avatarInitial(nm)}
                    </span>
                  );
                })}
                {(members ?? []).length > 6 && (
                  <span className="text-[11px] text-faint pl-[10px] tabular-nums">+{(members ?? []).length - 6}</span>
                )}
              </span>
            </>
          )}
        </button>
        {membersOpen && (
          <div className="px-[8px] pb-[10px]">
            {membersLoading ? (
              <div className="flex justify-center p-[12px]">
                <Spinner />
              </div>
            ) : (
              (members ?? []).map((m: RealmMember) => {
                const nm = m.username || m.user_id.slice(0, 8);
                // 同时是好友的成员:借好友列表(store 轮询维护在线/活动)展示在线状态。
                const friend = friendById.get(m.user_id);
                const isFriend = friend !== undefined;
                return (
                  <div
                    key={m.user_id}
                    className="flex items-center gap-[10px] px-[8px] py-[6px] group hover:bg-panel-2 transition-[background-color] duration-[var(--dur)] ease-app"
                  >
                    <PersonTile name={nm} online={friend?.online} pip={isFriend} />
                    <div className="flex flex-col min-w-0 flex-1">
                      <span className="text-[13px] text-fg truncate">
                        {nm}
                        {m.user_id === myId && <span className="text-muted"> {t("realm.you")}</span>}
                      </span>
                      {friend?.online && (
                        <span className="text-[11px] text-accent truncate">
                          {friend.activity ? t("friend.playing", { name: friend.activity ?? "" }) : t("friend.idle")}
                        </span>
                      )}
                      <span className="text-[11px] text-faint truncate">
                        {m.synced_version > 0 ? t("realm.syncedTo", { version: m.synced_version }) : t("realm.notSynced")}
                      </span>
                    </div>
                    {m.user_id !== myId && friendIds.has(m.user_id) && <Tag>{t("realm.friendTag")}</Tag>}
                    <Tag>{roleLabel(m.role)}</Tag>
                    {isOwner && m.role !== "owner" && (
                      <div className="flex items-center gap-[6px] shrink-0">
                        <button
                          className="text-[12px] text-muted hover:text-fg bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy}
                          onClick={() => void setRole(m.user_id, m.role === "member" ? "admin" : "member")}
                        >
                          {m.role === "member" ? t("realm.promote") : t("realm.demote")}
                        </button>
                        <button
                          className="text-[12px] text-danger-text hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy}
                          onClick={() => void removeMember(m.user_id)}
                        >
                          {t("realm.removeMember")}
                        </button>
                      </div>
                    )}
                  </div>
                );
              })
            )}
          </div>
        )}
      </Panel>

      {/* 邀请好友(owner/admin · 社交开启时):不折叠,直接一个搜索框,输入才出名字 */}
      {canPush && socialOn && kobeSignedIn && (
        <Panel variant="sunken" className="p-[16px] flex flex-col gap-[8px]">
          <span className="text-[12px] text-sub font-display tracking-[0.5px]">{t("realm.inviteTitle")}</span>
          <input
            className="h-[32px] px-[10px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            type="text"
            placeholder={t("realm.inviteSearchPlaceholder")}
            value={inviteQuery}
            onChange={(e) => setInviteQuery(e.currentTarget.value)}
          />
          {inviteQuery.trim().length > 0 &&
            (inviteMatches.length > 0 ? (
              <div className="flex flex-col gap-[2px] max-h-[200px] overflow-y-auto">
                {inviteMatches.map((f) => {
                  const nm = f.username || f.id.slice(0, 8);
                  return (
                    <div key={f.id} className={clsx("flex items-center gap-[10px] px-[8px] py-[6px]", { "opacity-70": !f.online })}>
                      <PersonTile name={nm} online={f.online} pip />
                      <div className="flex flex-col min-w-0 flex-1">
                        <span className="text-[13px] text-fg truncate">{nm}</span>
                        <span className="text-[11px] text-faint truncate">
                          {f.online
                            ? f.activity
                              ? t("friend.playing", { name: f.activity })
                              : t("friend.idle")
                            : t("friend.offline")}
                        </span>
                      </div>
                      {memberIds.has(f.id) ? (
                        <span className="text-[12px] text-faint">{t("realm.invited")}</span>
                      ) : (
                        <button
                          className="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                          disabled={busy}
                          onClick={() => void invite(f.id)}
                        >
                          {t("realm.invite")}
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            ) : (
              <p className="text-[12px] text-faint px-[2px]">{t("friend.noResults")}</p>
            ))}
        </Panel>
      )}

      {/* 退出 / 解散 */}
      <div className="flex justify-end">
        <Button variant="danger" disabled={busy} onClick={() => setConfirmKind(isOwner ? "disband" : "leave")}>
          {isOwner ? t("realm.disband") : t("realm.leave")}
        </Button>
      </div>

      <Dialog
        open={confirmKind !== null}
        onClose={() => setConfirmKind(null)}
        label={confirmKind === "disband" ? t("realm.disband") : t("realm.leave")}
      >
        <div className="p-[20px] flex flex-col gap-[16px]">
          <p className="text-[14px] text-fg leading-[1.6]">
            {confirmKind === "disband"
              ? t("realm.confirmDisband", { name: summary?.name ?? instance.name })
              : t("realm.confirmLeave", { name: summary?.name ?? instance.name })}
          </p>
          <div className="flex justify-end gap-[8px]">
            <Button variant="ghost" onClick={() => setConfirmKind(null)}>
              {t("realm.cancel")}
            </Button>
            <Button variant="danger" disabled={busy} onClick={() => void (confirmKind === "disband" ? doDisband() : doLeave())}>
              {confirmKind === "disband" ? t("realm.disband") : t("realm.leave")}
            </Button>
          </div>
        </div>
      </Dialog>

      {/* 未登录兜底(理论上能进到这里说明实例有 realm 绑定但会话过期) */}
      {!kobeSignedIn && (
        <p className="text-[12px] text-muted">
          {t("realm.needLogin")}{" "}
          <button
            className="text-accent hover:underline bg-transparent border-none cursor-pointer p-0"
            onClick={() => setCurrentPage("settings")}
          >
            {t("realm.goLogin")}
          </button>
        </p>
      )}
    </div>
  );
}

export default RealmPanel;
