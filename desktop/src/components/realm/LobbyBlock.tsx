import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { Button } from "../Button";
import { Panel } from "../Panel";
import { toast } from "../Toast";
import { Select } from "../Select";
import { api } from "../../ipc/api";
import { playInstance, useAppStore } from "../../store";
import { useAsync } from "../../util/useAsync";
import { t } from "../../i18n";
import type { LobbyStatus, RealmHost } from "../../ipc/bindings";

export function LobbyBlock({ realmId, instanceId }: { realmId: string; instanceId: string }) {
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
