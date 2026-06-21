import { Component, createSignal, createEffect, Show } from "solid-js";
import { Dialog } from "./Dialog";
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { activeRoot } from "../store";
import type { InstanceConfig, InstanceSummary } from "../ipc/types";

/**
 * InstanceManageDialog —— 单实例设置编辑(名字/内存/Java/JVM 参数/窗口)。
 * 读 daemon 的 get_instance_config,改一项即 set_instance_config 持久化。
 * (后续 Mods / 资源包 标签页接入时,这里加一层 tab。)
 */

const FIELD =
  "h-[34px] px-[12px] rounded-ctl border border-n-6 bg-n-2 text-fg text-[13px] outline-none " +
  "transition-colors duration-150 focus:border-a-4";
const LABEL = "text-[12px] text-dim";

export const InstanceManageDialog: Component<{
  open: boolean;
  instance: InstanceSummary | null;
  onClose: () => void;
  onChanged?: () => void;
}> = (props) => {
  const [cfg, setCfg] = createSignal<InstanceConfig | null>(null);

  // 打开/切换实例时拉配置;关闭时清空。
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
    }
  });

  // 改一项并立即持久化到该实例的 instance.json。
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

  return (
    <Dialog
      open={props.open}
      onClose={props.onClose}
      label="实例设置"
      contentClass="w-[480px] max-w-[calc(100vw-48px)] bg-card rounded-card shadow-card overflow-hidden focus:outline-none"
    >
      <div class="p-[20px] flex flex-col gap-[14px] max-h-[calc(100vh-120px)] overflow-y-auto">
        <div class="text-[15px] font-bold text-fg">
          实例设置 ·{" "}
          <span class="text-dim font-normal">
            {props.instance?.name || props.instance?.id}
          </span>
        </div>

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
                <span class={LABEL}>额外 JVM 参数(空格分隔,如 -XX:+UseG1GC)</span>
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

        <div class="flex justify-end mt-[4px]">
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
