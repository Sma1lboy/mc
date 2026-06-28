import { Component, createEffect, createResource, createSignal, For, Show } from "solid-js";
import { Spinner } from "./Spinner";
import { EmptyState } from "./EmptyState";
import { ErrorState } from "./ErrorState";
import { Panel } from "./Panel";
import { toast } from "./Toast";
import { ACCENT_BTN_COMPACT } from "./styles";
import { api } from "../ipc/api";
import { activeRoot, isLaunching, isRunning, playInstance } from "../store";
import { t } from "../i18n";
import type { InstanceSummary, ServerStatus } from "../ipc/types";

/**
 * ServersPanel —— 实例的多人服务器列表(读 game_dir/servers.dat)。每条给图标 + 名称 + 地址,
 * 状态(在线/离线点、人数、延迟、MOTD,挂载时并发 ping 拿到),以及「进入游戏」直接带着该地址
 * 启动(一次性服务器覆盖,不改实例配置);底部可手动添加一条写回 servers.dat。
 */
const ServersPanel: Component<{ instance: InstanceSummary }> = (props) => {
  const [servers, { refetch }] = createResource(
    () => props.instance.id,
    (id) => api.instanceServers(activeRoot(), id),
  );
  const busy = () => isLaunching(props.instance.id) || isRunning(props.instance.id);

  // address -> 最近一次 ping 的状态。undefined 表示尚未 ping(或正在 ping)。
  const [statuses, setStatuses] = createSignal<Record<string, ServerStatus>>({});
  const [pinging, setPinging] = createSignal(false);

  /** 以有界并发 ping 一组地址,结果就绪即写回(不等全部完成)。 */
  async function pingAll(addresses: string[]) {
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
  }

  // 列表就绪 / 变化时自动 ping 一次。
  createEffect(() => {
    const list = servers();
    if (list) void pingAll(list.map((s) => s.address));
  });

  const [addName, setAddName] = createSignal("");
  const [addAddress, setAddAddress] = createSignal("");
  const [adding, setAdding] = createSignal(false);
  const canAdd = () => !!addAddress().trim() && !adding();

  async function addServer() {
    if (!canAdd()) return;
    setAdding(true);
    try {
      await api.addInstanceServer(activeRoot(), props.instance.id, addName().trim(), addAddress().trim());
      setAddName("");
      setAddAddress("");
      await refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.serverAddFailed", { error: String(e) }) });
    } finally {
      setAdding(false);
    }
  }

  const inputClass =
    "h-[28px] bg-panel-2 text-fg text-[12px] px-[10px] rounded-none shadow-input border-none outline-none placeholder:text-faint focus:shadow-pressed";

  return (
    <div class="h-full overflow-y-auto p-[16px] flex flex-col gap-[12px]">
      <Show when={!servers.loading} fallback={<div class="flex justify-center p-[24px]"><Spinner /></div>}>
        <Show
          when={!servers.error}
          fallback={<ErrorState message={t("instance.serversError")} onRetry={() => void refetch()} />}
        >
          <Show
            when={(servers() ?? []).length > 0}
            fallback={<EmptyState title={t("instance.serversEmpty")} />}
          >
            {/* 顶栏:手动刷新所有服务器状态。 */}
            <div class="flex items-center justify-end">
              <button
                class="text-[11px] text-muted hover:text-fg disabled:opacity-50 flex items-center gap-[5px]"
                disabled={pinging()}
                onClick={() => void pingAll((servers() ?? []).map((s) => s.address))}
                title={t("instance.serverRefresh")}
              >
                <Show when={pinging()} fallback={
                  <svg class="w-[13px] h-[13px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6" />
                  </svg>
                }>
                  <Spinner />
                </Show>
                {pinging() ? t("instance.serverPinging") : t("instance.serverRefresh")}
              </button>
            </div>
            <div class="flex flex-col gap-[8px]">
              <For each={servers()!}>
                {(s) => {
                  const status = () => statuses()[s.address];
                  const pendingPing = () => status() === undefined;
                  const online = () => status()?.online === true;
                  return (
                    <Panel variant="sunken" class="flex items-center gap-[12px] bg-panel px-[12px] py-[9px]">
                      <Panel variant="input" class="w-[36px] h-[36px] overflow-hidden bg-panel-2 grid place-items-center shrink-0">
                        <Show
                          when={s.icon}
                          fallback={
                            <svg class="w-[18px] h-[18px] text-muted" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
                              <circle cx="12" cy="12" r="9" />
                              <path d="M3 12h18M12 3a15 15 0 0 1 0 18M12 3a15 15 0 0 0 0 18" />
                            </svg>
                          }
                        >
                          <img
                            src={`data:image/png;base64,${s.icon}`}
                            alt=""
                            class="w-full h-full object-cover"
                            style="image-rendering:pixelated"
                          />
                        </Show>
                      </Panel>
                      <div class="min-w-0 flex-1">
                        <div class="flex items-center gap-[8px] min-w-0">
                          <div class="text-[13px] font-semibold text-fg truncate">{s.name || s.address}</div>
                          {/* 在线/离线点 + 人数 + 延迟。 */}
                          <Show when={!pendingPing()}>
                            <span
                              class="w-[7px] h-[7px] rounded-full shrink-0"
                              classList={{ "bg-emerald-500": online(), "bg-faint": !online() }}
                              title={online() ? t("instance.serverOnline") : t("instance.serverOffline")}
                            />
                            <Show
                              when={online()}
                              fallback={<span class="text-[11px] text-faint shrink-0">{t("instance.serverOffline")}</span>}
                            >
                              <Show when={status()!.players_max != null}>
                                <span class="text-[11px] text-muted shrink-0">
                                  {t("instance.serverPlayers", {
                                    online: status()!.players_online ?? 0,
                                    max: status()!.players_max ?? 0,
                                  })}
                                </span>
                              </Show>
                              <Show when={status()!.latency_ms != null}>
                                <span class="text-[11px] text-muted shrink-0">
                                  {t("instance.serverLatency", { ms: status()!.latency_ms ?? 0 })}
                                </span>
                              </Show>
                            </Show>
                          </Show>
                        </div>
                        <div class="text-[11px] text-muted truncate">
                          <Show when={online() && status()!.motd} fallback={s.address}>
                            {status()!.motd}
                          </Show>
                        </div>
                      </div>
                      <button
                        class={ACCENT_BTN_COMPACT}
                        disabled={busy()}
                        onClick={() => void playInstance(props.instance.id, s.address)}
                        title={t("instance.serverJoinHint", { address: s.address })}
                      >
                        {t("instance.serverJoin")}
                      </button>
                    </Panel>
                  );
                }}
              </For>
            </div>
          </Show>
        </Show>
      </Show>

      {/* 手动添加一条服务器(写回 servers.dat;地址必填,名称可空)。 */}
      <Panel variant="sunken" class="flex items-center gap-[8px] bg-panel px-[12px] py-[9px]">
        <input
          class={`${inputClass} w-[120px] shrink-0`}
          value={addName()}
          onInput={(e) => setAddName(e.currentTarget.value)}
          placeholder={t("instance.serverNamePlaceholder")}
        />
        <input
          class={`${inputClass} flex-1 min-w-0`}
          value={addAddress()}
          onInput={(e) => setAddAddress(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void addServer();
            }
          }}
          placeholder={t("instance.serverAddressPlaceholder")}
        />
        <button class={ACCENT_BTN_COMPACT} disabled={!canAdd()} onClick={() => void addServer()}>
          {t("instance.serverAdd")}
        </button>
      </Panel>
    </div>
  );
};

export default ServersPanel;
