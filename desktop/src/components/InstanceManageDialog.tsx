import { Component, createSignal, createResource, createEffect, For, Show } from "solid-js";
import { Dialog } from "./Dialog";
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { activeRoot } from "../store";
import type { InstanceConfig, InstanceSummary, ModInfo } from "../ipc/types";

/**
 * InstanceManageDialog —— 单实例管理:设置(名字/内存/Java/JVM/窗口)+ Mods(启停/删除)。
 * 设置改一项即 set_instance_config 持久化;Mods 用 set_mod_enabled / delete_mod。
 */

const FIELD =
  "h-[34px] px-[12px] rounded-ctl border border-n-6 bg-n-2 text-fg text-[13px] outline-none " +
  "transition-colors duration-150 focus:border-a-4";
const LABEL = "text-[12px] text-dim";
const TAB =
  "px-[14px] py-[7px] text-[13px] font-semibold cursor-pointer border-b-2 border-b-transparent " +
  "text-n-6 hover:text-n-8 transition-colors duration-150";
const TAB_ACTIVE = "!text-a-6 !border-b-a-5";

type Tab = "settings" | "mods";

export const InstanceManageDialog: Component<{
  open: boolean;
  instance: InstanceSummary | null;
  onClose: () => void;
  onChanged?: () => void;
}> = (props) => {
  const [tab, setTab] = createSignal<Tab>("settings");
  const [cfg, setCfg] = createSignal<InstanceConfig | null>(null);

  // 打开/切换实例时拉配置 + 复位到设置页;关闭时清空。
  createEffect(() => {
    const inst = props.instance;
    if (props.open && inst) {
      setCfg(null);
      api
        .getInstanceConfig(activeRoot(), inst.id)
        .then(setCfg)
        .catch((e) => toast({ type: "error", message: `读取配置失败:${e}` }));
    } else if (!props.open) {
      setCfg(null);
      setTab("settings");
    }
  });

  // Mods:仅在 Mods 标签 + 弹窗打开时拉取。
  const [mods, { refetch: refetchMods }] = createResource(
    () => (props.open && props.instance && tab() === "mods" ? props.instance.id : false),
    (id) => api.instanceMods(activeRoot(), id as string),
  );

  function patch(p: Partial<InstanceConfig>) {
    const cur = cfg();
    const inst = props.instance;
    if (!cur || !inst) return;
    const next = { ...cur, ...p };
    setCfg(next);
    void api
      .setInstanceConfig(activeRoot(), inst.id, next)
      .then(() => props.onChanged?.())
      .catch((e) => toast({ type: "error", message: `保存失败:${e}` }));
  }

  async function toggleMod(m: ModInfo, enabled: boolean) {
    const inst = props.instance;
    if (!inst) return;
    try {
      await api.setModEnabled(activeRoot(), inst.id, m.file_name, enabled);
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: `操作失败:${e}` });
    }
  }

  async function removeMod(m: ModInfo) {
    const inst = props.instance;
    if (!inst) return;
    try {
      await api.deleteMod(activeRoot(), inst.id, m.file_name);
      toast({ type: "success", message: `已删除 ${m.name}` });
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: `删除失败:${e}` });
    }
  }

  return (
    <Dialog
      open={props.open}
      onClose={props.onClose}
      label="实例管理"
      contentClass="w-[520px] max-w-[calc(100vw-48px)] bg-card rounded-card shadow-card overflow-hidden focus:outline-none"
    >
      <div class="flex flex-col max-h-[calc(100vh-100px)]">
        <div class="px-[20px] pt-[18px] text-[15px] font-bold text-fg">
          {props.instance?.name || props.instance?.id}
        </div>

        <div class="flex gap-[4px] px-[16px] border-b border-n-3 mt-[10px]">
          <button class={`${TAB} ${tab() === "settings" ? TAB_ACTIVE : ""}`} onClick={() => setTab("settings")}>
            设置
          </button>
          <button class={`${TAB} ${tab() === "mods" ? TAB_ACTIVE : ""}`} onClick={() => setTab("mods")}>
            Mods
          </button>
        </div>

        <div class="p-[20px] flex flex-col gap-[14px] overflow-y-auto">
          {/* ---- 设置 ---- */}
          <Show when={tab() === "settings"}>
            <Show
              when={cfg()}
              fallback={
                <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
                  <Spinner size={16} /> 读取配置中…
                </div>
              }
            >
              {(c) => (
                <>
                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>名称</span>
                    <input
                      class={FIELD}
                      value={c().name ?? ""}
                      onChange={(e) => patch({ name: e.currentTarget.value || null })}
                    />
                  </label>

                  <div class="flex flex-col gap-[5px]">
                    <span class={LABEL}>最大内存 {c().memory_mb} MiB</span>
                    <input
                      class="w-full accent-[var(--a-4)]"
                      type="range"
                      min="512"
                      max="16384"
                      step="256"
                      value={c().memory_mb}
                      onInput={(e) => setCfg({ ...c(), memory_mb: +e.currentTarget.value })}
                      onChange={(e) => patch({ memory_mb: +e.currentTarget.value })}
                    />
                  </div>

                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>Java 路径(留空 = 跟随全局/自动)</span>
                    <input
                      class={FIELD}
                      placeholder="自动 / 全局设置"
                      value={c().java_path ?? ""}
                      onChange={(e) => patch({ java_path: e.currentTarget.value || null })}
                    />
                  </label>

                  <label class="flex flex-col gap-[5px]">
                    <span class={LABEL}>额外 JVM 参数(空格分隔)</span>
                    <input
                      class={FIELD}
                      value={c().jvm_args.join(" ")}
                      onChange={(e) =>
                        patch({ jvm_args: e.currentTarget.value.split(/\s+/).filter(Boolean) })
                      }
                    />
                  </label>

                  <div class="flex gap-[12px]">
                    <label class="flex-1 flex flex-col gap-[5px]">
                      <span class={LABEL}>窗口宽</span>
                      <input
                        class={FIELD}
                        type="number"
                        placeholder="默认"
                        value={c().width ?? ""}
                        onChange={(e) =>
                          patch({ width: e.currentTarget.value ? +e.currentTarget.value : null })
                        }
                      />
                    </label>
                    <label class="flex-1 flex flex-col gap-[5px]">
                      <span class={LABEL}>窗口高</span>
                      <input
                        class={FIELD}
                        type="number"
                        placeholder="默认"
                        value={c().height ?? ""}
                        onChange={(e) =>
                          patch({ height: e.currentTarget.value ? +e.currentTarget.value : null })
                        }
                      />
                    </label>
                  </div>

                  <label class="flex items-center justify-between text-fg text-[13px]">
                    <span>全屏启动</span>
                    <input
                      type="checkbox"
                      class="w-[16px] h-[16px] accent-[var(--a-4)] cursor-pointer"
                      checked={c().fullscreen}
                      onChange={(e) => patch({ fullscreen: e.currentTarget.checked })}
                    />
                  </label>
                </>
              )}
            </Show>
          </Show>

          {/* ---- Mods ---- */}
          <Show when={tab() === "mods"}>
            <Show
              when={!mods.loading}
              fallback={
                <div class="flex items-center gap-[10px] text-dim text-[13px] py-[12px]">
                  <Spinner size={16} /> 扫描 mods…
                </div>
              }
            >
              <Show
                when={(mods() ?? []).length > 0}
                fallback={<div class="text-dim text-[13px] py-[12px]">该实例还没有 mod。</div>}
              >
                <div class="flex flex-col gap-[6px]">
                  <For each={mods()}>
                    {(m) => (
                      <div
                        class="flex items-center gap-[10px] py-[8px] px-[10px] rounded-ctl bg-n-3"
                        classList={{ "opacity-55": !m.enabled }}
                      >
                        <div class="flex-1 min-w-0">
                          <div class="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                            {m.name}
                          </div>
                          <div class="text-[11px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                            {[m.version, m.loader, m.file_name].filter(Boolean).join(" · ")}
                          </div>
                        </div>
                        <label class="flex items-center gap-[5px] text-[11px] text-dim cursor-pointer shrink-0">
                          <input
                            type="checkbox"
                            class="w-[15px] h-[15px] accent-[var(--a-4)] cursor-pointer"
                            checked={m.enabled}
                            onChange={(e) => toggleMod(m, e.currentTarget.checked)}
                          />
                          启用
                        </label>
                        <button
                          class="shrink-0 text-[12px] text-[#e5848a] px-[8px] py-[4px] rounded-xs cursor-pointer hover:bg-[rgba(229,132,138,0.14)]"
                          onClick={() => removeMod(m)}
                        >
                          删除
                        </button>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Show>
          </Show>
        </div>

        <div class="flex justify-end px-[20px] py-[14px] border-t border-n-3">
          <button
            class="h-[34px] px-[16px] border border-n-6 rounded-ctl bg-n-4 text-fg text-[13px] cursor-pointer transition-colors duration-150 hover:bg-n-5"
            onClick={props.onClose}
          >
            完成
          </button>
        </div>
      </div>
    </Dialog>
  );
};

export default InstanceManageDialog;
