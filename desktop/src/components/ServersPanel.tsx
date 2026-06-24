import { Component, createResource, For, Show } from "solid-js";
import { Spinner } from "./Spinner";
import { EmptyState } from "./EmptyState";
import { ErrorState } from "./ErrorState";
import { ACCENT_BTN_COMPACT } from "./styles";
import { api } from "../ipc/api";
import { activeRoot, isLaunching, isRunning, playInstance } from "../store";
import { t } from "../i18n";
import type { InstanceSummary } from "../ipc/types";

/**
 * ServersPanel —— 实例的多人服务器列表(读 game_dir/servers.dat)。每条给图标 + 名称 + 地址,
 * 以及「进入游戏」直接带着该地址启动(一次性服务器覆盖,不改实例配置)。
 */
const ServersPanel: Component<{ instance: InstanceSummary }> = (props) => {
  const [servers, { refetch }] = createResource(
    () => props.instance.id,
    (id) => api.instanceServers(activeRoot(), id),
  );
  const busy = () => isLaunching(props.instance.id) || isRunning(props.instance.id);

  return (
    <div class="h-full overflow-y-auto p-[16px]">
      <Show when={!servers.loading} fallback={<div class="flex justify-center p-[24px]"><Spinner /></div>}>
        <Show
          when={!servers.error}
          fallback={<ErrorState message={t("instance.serversError")} onRetry={() => void refetch()} />}
        >
          <Show
            when={(servers() ?? []).length > 0}
            fallback={<EmptyState title={t("instance.serversEmpty")} />}
          >
            <div class="flex flex-col gap-[8px]">
              <For each={servers()!}>
                {(s) => (
                  <div class="flex items-center gap-[12px] rounded-ctl glass-card px-[12px] py-[9px]">
                    <div class="w-[36px] h-[36px] rounded-[6px] overflow-hidden bg-glass-hover grid place-items-center shrink-0">
                      <Show
                        when={s.icon}
                        fallback={
                          <svg class="w-[18px] h-[18px] text-dim" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
                    </div>
                    <div class="min-w-0 flex-1">
                      <div class="text-[13px] font-semibold text-fg truncate">{s.name || s.address}</div>
                      <div class="text-[11px] text-dim truncate">{s.address}</div>
                    </div>
                    <button
                      class={ACCENT_BTN_COMPACT}
                      disabled={busy()}
                      onClick={() => void playInstance(props.instance.id, s.address)}
                      title={t("instance.serverJoinHint", { address: s.address })}
                    >
                      {t("instance.serverJoin")}
                    </button>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </Show>
      </Show>
    </div>
  );
};

export default ServersPanel;
