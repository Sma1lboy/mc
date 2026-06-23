import { Component, createResource, Show } from "solid-js";
import { InstanceManageDialog, toast } from "../components";
import { PlayButton } from "../components/PlayButton";
import { formatRelativeTime } from "../components/format";
import { api } from "../ipc/api";
import { activeRoot, isRunning, currentInstanceId, closeInstance, openInstance } from "../store";

/**
 * InstanceDetail —— 实例详情页(替代旧的管理弹窗):
 *   顶部头部(返回 / 图标 / 名称 / 版本·加载器 / 大 Play)
 *   下方复用 InstanceManageDialog(embedded:tabs 设置/Mods/资源包/光影/数据包/存档/截图)。
 * 由 store.openInstance(id) 进入,closeInstance() 返回来源页。
 */
const InstanceDetail: Component = () => {
  const [data, { refetch }] = createResource(
    () => [activeRoot(), currentInstanceId()] as const,
    async ([root, id]) => {
      if (!id) return null;
      const list = await api.listInstances(root);
      return list.find((i) => i.id === id) ?? null;
    },
  );
  const inst = () => data() ?? null;

  async function play() {
    const i = inst();
    if (!i) return;
    if (isRunning(i.id)) {
      try {
        await api.stopInstance(i.id);
      } catch (e) {
        toast({ type: "error", message: `停止失败:${e}` });
      }
      return;
    }
    try {
      await api.launchInstance(activeRoot(), i.id, "Player", false);
      toast({ type: "success", message: "已启动" });
    } catch (e) {
      toast({ type: "error", message: `启动失败:${e}` });
    }
  }

  const loaderLabel = () => {
    const i = inst();
    if (!i) return "";
    const l = i.loader;
    const cap = l ? l.charAt(0).toUpperCase() + l.slice(1) : "";
    return `${cap} ${i.mc_version}`.trim();
  };
  const playedLabel = () => {
    const i = inst();
    if (!i) return "";
    const rel = formatRelativeTime(i.last_played ?? 0);
    return rel === "never" ? "从未游玩" : `上次 ${rel}`;
  };

  return (
    <div class="flex flex-col h-full min-h-0 overflow-hidden">
      {/* 头部 */}
      <div class="flex items-center gap-[14px] px-[28px] pt-[20px] pb-[14px] border-b border-glass-divider">
        <button
          class="shrink-0 w-[34px] h-[34px] grid place-items-center rounded-ctl border-none bg-transparent text-dim cursor-pointer transition-colors duration-150 hover:bg-glass-hover hover:text-fg"
          onClick={closeInstance}
          aria-label="返回"
          title="返回"
        >
          <svg class="w-[18px] h-[18px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <path d="m14 6-6 6 6 6" />
          </svg>
        </button>

        <Show
          when={inst()}
          fallback={<div class="text-dim text-[14px] py-[8px]">载入中…</div>}
        >
          {(i) => (
            <>
              <div class="relative shrink-0 w-[52px] h-[52px] rounded-ctl overflow-hidden grid place-items-center bg-gradient-to-br from-a-3 to-a-5 text-white font-bold text-[22px] uppercase select-none">
                <Show when={i().icon} fallback={<span>{(i().name || i().id).charAt(0)}</span>}>
                  <img src={i().icon!} alt="" width="52" height="52" class="w-full h-full object-cover" />
                </Show>
                <Show when={isRunning(i().id)}>
                  <span class="absolute right-[3px] bottom-[3px] w-[12px] h-[12px] rounded-full bg-a-5 shadow-[0_0_0_2px_var(--bg-card)]" title="运行中" />
                </Show>
              </div>
              <div class="flex-1 min-w-0">
                <div class="text-[20px] font-bold text-fg whitespace-nowrap overflow-hidden text-ellipsis" title={i().name || i().id}>
                  {i().name || i().id}
                </div>
                <div class="text-[12px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                  {loaderLabel()} · {playedLabel()}
                </div>
              </div>
              <PlayButton running={isRunning(i().id)} onClick={play} />
            </>
          )}
        </Show>
      </div>

      {/* tabs + 内容(复用管理面板的 embedded 模式) */}
      <div class="flex-1 min-h-0 overflow-hidden">
        <Show when={inst()}>
          {(i) => (
            <InstanceManageDialog
              embedded
              open
              instance={i()}
              onChanged={() => void refetch()}
              onCopied={(newId) => openInstance(newId)}
            />
          )}
        </Show>
      </div>
    </div>
  );
};

export default InstanceDetail;
