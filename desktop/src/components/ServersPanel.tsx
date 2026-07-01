import { useCallback, useEffect, useState } from "react";
import clsx from "clsx";
import { Spinner } from "./Spinner";
import { EmptyState } from "./EmptyState";
import { ErrorState } from "./ErrorState";
import { Panel } from "./Panel";
import { toast } from "./Toast";
import { ACCENT_BTN_COMPACT } from "./styles";
import { api } from "../ipc/api";
import { activeRoot, isLaunching, isRunning, playInstance, useAppStore } from "../store";
import { useAsync } from "../util/useAsync";
import { t, useLang } from "../i18n";
import type { InstanceSummary, ServerStatus } from "../ipc/types";

/**
 * ServersPanel —— 实例的多人服务器列表(读 game_dir/servers.dat)。每条给图标 + 名称 + 地址,
 * 状态(在线/离线点、人数、延迟、MOTD,挂载时并发 ping 拿到),以及「进入游戏」直接带着该地址
 * 启动(一次性服务器覆盖,不改实例配置);底部可手动添加一条写回 servers.dat。
 */
const ServersPanel = (props: { instance: InstanceSummary }): React.ReactElement => {
  useLang();
  const { instance } = props;
  const { data: servers, loading, error, refetch } = useAsync(
    () => api.instanceServers(activeRoot(), instance.id),
    [instance.id],
  );
  // 订阅运行/启动集合,使运行态变化即时反映在按钮禁用上。
  useAppStore((s) => s.runningIds);
  useAppStore((s) => s.launchingIds);
  const busy = isLaunching(instance.id) || isRunning(instance.id);

  // address -> 最近一次 ping 的状态。undefined 表示尚未 ping(或正在 ping)。
  const [statuses, setStatuses] = useState<Record<string, ServerStatus>>({});
  const [pinging, setPinging] = useState(false);

  /** 以有界并发 ping 一组地址,结果就绪即写回(不等全部完成)。 */
  const pingAll = useCallback(async (addresses: string[]) => {
    if (addresses.length === 0) return;
    setPinging(true);
    setStatuses({});
    const queue = [...addresses];
    const limit = Math.min(6, queue.length);
    const worker = async () => {
      for (;;) {
        const addr = queue.shift();
        if (addr === undefined) return;
        const status = await api.pingServer(addr);
        setStatuses((prev) => ({ ...prev, [addr]: status }));
      }
    };
    try {
      await Promise.all(Array.from({ length: limit }, worker));
    } finally {
      setPinging(false);
    }
  }, []);

  // 列表就绪 / 变化时自动 ping 一次。
  useEffect(() => {
    if (servers) void pingAll(servers.map((s) => s.address));
  }, [servers, pingAll]);

  const [addName, setAddName] = useState("");
  const [addAddress, setAddAddress] = useState("");
  const [adding, setAdding] = useState(false);
  const canAdd = !!addAddress.trim() && !adding;

  async function addServer() {
    if (!canAdd) return;
    setAdding(true);
    try {
      await api.addInstanceServer(activeRoot(), instance.id, addName.trim(), addAddress.trim());
      setAddName("");
      setAddAddress("");
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.serverAddFailed", { error: String(e) }) });
    } finally {
      setAdding(false);
    }
  }

  const inputClass =
    "h-[28px] bg-panel-2 text-fg text-[12px] px-[10px] rounded-none shadow-input border-none outline-none placeholder:text-faint focus:shadow-pressed";

  const list = servers ?? [];

  return (
    <div className="h-full overflow-y-auto p-[16px] flex flex-col gap-[12px]">
      {loading ? (
        <div className="flex justify-center p-[24px]">
          <Spinner />
        </div>
      ) : error ? (
        <ErrorState message={t("instance.serversError")} onRetry={() => refetch()} />
      ) : list.length === 0 ? (
        <EmptyState title={t("instance.serversEmpty")} />
      ) : (
        <>
          {/* 顶栏:手动刷新所有服务器状态。 */}
          <div className="flex items-center justify-end">
            <button
              className="text-[11px] text-muted hover:text-fg disabled:opacity-50 flex items-center gap-[5px]"
              disabled={pinging}
              onClick={() => void pingAll(list.map((s) => s.address))}
              title={t("instance.serverRefresh")}
            >
              {pinging ? (
                <Spinner />
              ) : (
                <svg className="w-[13px] h-[13px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6" />
                </svg>
              )}
              {pinging ? t("instance.serverPinging") : t("instance.serverRefresh")}
            </button>
          </div>
          <div className="flex flex-col gap-[8px]">
            {list.map((s) => {
              const status = statuses[s.address];
              const pendingPing = status === undefined;
              const online = status?.online === true;
              return (
                <Panel key={s.address} variant="sunken" className="flex items-center gap-[12px] bg-panel px-[12px] py-[9px]">
                  <Panel variant="input" className="w-[36px] h-[36px] overflow-hidden bg-panel-2 grid place-items-center shrink-0">
                    {s.icon ? (
                      <img
                        src={`data:image/png;base64,${s.icon}`}
                        alt=""
                        className="w-full h-full object-cover"
                        style={{ imageRendering: "pixelated" }}
                      />
                    ) : (
                      <svg className="w-[18px] h-[18px] text-muted" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
                        <circle cx="12" cy="12" r="9" />
                        <path d="M3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18" />
                      </svg>
                    )}
                  </Panel>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-[8px] min-w-0">
                      <div className="text-[13px] font-semibold text-fg truncate">{s.name || s.address}</div>
                      {/* 在线/离线点 + 人数 + 延迟。 */}
                      {!pendingPing && (
                        <>
                          <span
                            className={clsx("w-[7px] h-[7px] rounded-full shrink-0", online ? "bg-emerald-500" : "bg-faint")}
                            title={online ? t("instance.serverOnline") : t("instance.serverOffline")}
                          />
                          {online ? (
                            <>
                              {status!.players_max != null && (
                                <span className="text-[11px] text-muted shrink-0">
                                  {t("instance.serverPlayers", {
                                    online: status!.players_online ?? 0,
                                    max: status!.players_max ?? 0,
                                  })}
                                </span>
                              )}
                              {status!.latency_ms != null && (
                                <span className="text-[11px] text-muted shrink-0">
                                  {t("instance.serverLatency", { ms: status!.latency_ms ?? 0 })}
                                </span>
                              )}
                            </>
                          ) : (
                            <span className="text-[11px] text-faint shrink-0">{t("instance.serverOffline")}</span>
                          )}
                        </>
                      )}
                    </div>
                    <div className="text-[11px] text-muted truncate">
                      {online && status!.motd ? status!.motd : s.address}
                    </div>
                  </div>
                  <button
                    className={ACCENT_BTN_COMPACT}
                    disabled={busy}
                    onClick={() => void playInstance(instance.id, s.address)}
                    title={t("instance.serverJoinHint", { address: s.address })}
                  >
                    {t("instance.serverJoin")}
                  </button>
                </Panel>
              );
            })}
          </div>
        </>
      )}

      {/* 手动添加一条服务器(写回 servers.dat;地址必填,名称可空)。 */}
      <Panel variant="sunken" className="flex items-center gap-[8px] bg-panel px-[12px] py-[9px]">
        <input
          className={`${inputClass} w-[120px] shrink-0`}
          value={addName}
          onChange={(e) => setAddName(e.currentTarget.value)}
          placeholder={t("instance.serverNamePlaceholder")}
        />
        <input
          className={`${inputClass} flex-1 min-w-0`}
          value={addAddress}
          onChange={(e) => setAddAddress(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void addServer();
            }
          }}
          placeholder={t("instance.serverAddressPlaceholder")}
        />
        <button className={ACCENT_BTN_COMPACT} disabled={!canAdd} onClick={() => void addServer()}>
          {t("instance.serverAdd")}
        </button>
      </Panel>
    </div>
  );
};

export default ServersPanel;
