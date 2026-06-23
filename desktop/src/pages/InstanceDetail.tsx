import { Component, createResource, createSignal, onCleanup, onMount, Show } from "solid-js";
import { InstanceManageDialog, toast, type InstanceRowData } from "../components";
import { PlayButton } from "../components/PlayButton";
import { Menu } from "../components/Menu";
import { formatRelativeTime } from "../components/format";
import { api } from "../ipc/api";
import { openInstanceDir, exportInstanceMrpack, deleteInstance } from "../util/instanceActions";
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
  // 进入「添加」浏览模式时整页让给复用的探索视图,隐藏头部(返回路径用视图内的「← 返回已安装」)。
  const [browsing, setBrowsing] = createSignal(false);

  // Esc 返回上一页(与详情页导航一致);浏览模式有自己的 Esc,正在输入文本时不抢。
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || browsing()) return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable))
        return;
      e.preventDefault();
      closeInstance();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

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

  // 把当前实例转成 InstanceRow / instanceActions 共用的行数据形状(导出整合包需要)。
  const toRowData = (): InstanceRowData | null => {
    const i = inst();
    if (!i) return null;
    return {
      id: i.id,
      name: i.name || i.id,
      mc_version: i.mc_version,
      loader: i.loader,
      loader_version: i.loader_version || undefined,
      icon: i.icon || undefined,
      last_played: i.last_played ?? 0,
      running: isRunning(i.id),
    };
  };

  async function copyCurrent() {
    const i = inst();
    if (!i) return;
    try {
      const newId = await api.copyInstance(activeRoot(), i.id, `${i.name || i.id} 副本`);
      toast({ type: "success", message: "已复制实例" });
      openInstance(newId);
    } catch (e) {
      toast({ type: "error", message: `复制失败:${e}` });
    }
  }

  async function onMenuAction(value: string) {
    const i = inst();
    const row = toRowData();
    if (!i || !row) return;
    if (value === "open") void openInstanceDir(activeRoot(), i.id);
    else if (value === "copy") await copyCurrent();
    else if (value === "export") void exportInstanceMrpack(activeRoot(), row);
    else if (value === "delete") {
      if (await deleteInstance(activeRoot(), { id: i.id, name: i.name || i.id })) closeInstance();
    }
  }

  return (
    <div class="flex flex-col h-full min-h-0 overflow-hidden">
      {/* 头部:浏览(添加)模式下隐藏,整页让给复用的探索视图。 */}
      <Show when={!browsing()}>
      <div class="flex flex-col gap-[12px] px-[28px] pt-[14px] pb-[14px] border-b border-glass-divider">
        {/* 返回:与其它详情页一致的文字返回(整行最上方),不再用漂浮的箭头按钮。 */}
        <button
          class="self-start inline-flex items-center gap-[3px] bg-transparent border-none text-dim text-[13px] cursor-pointer py-[2px] px-0 transition-colors duration-150 hover:text-fg"
          onClick={closeInstance}
          aria-label="返回"
        >
          <svg class="w-[16px] h-[16px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m14 6-6 6 6 6" />
          </svg>
          返回
        </button>

        <Show
          when={inst()}
          fallback={<div class="text-dim text-[14px] py-[8px]">载入中…</div>}
        >
          {(i) => (
            <div class="flex items-center gap-[14px]">
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
              <Menu.Root positioning={{ placement: "bottom-end" }} onSelect={(d: { value: string }) => void onMenuAction(d.value)}>
                <Menu.Trigger
                  class="inline-flex items-center justify-center w-[34px] h-[34px] border-none bg-transparent text-dim rounded-ctl cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-glass-hover hover:text-fg data-[state=open]:bg-glass-hover data-[state=open]:text-fg"
                  aria-label="更多操作"
                >
                  <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
                    <circle cx="8" cy="3" r="1.5" />
                    <circle cx="8" cy="8" r="1.5" />
                    <circle cx="8" cy="13" r="1.5" />
                  </svg>
                </Menu.Trigger>
                <Menu.Content>
                  <Menu.Item value="open">打开游戏目录</Menu.Item>
                  <Menu.Item value="copy">复制实例</Menu.Item>
                  <Menu.Item value="export">导出整合包(.mrpack)</Menu.Item>
                  <Menu.Separator />
                  <Menu.Item value="delete" danger>
                    删除实例
                  </Menu.Item>
                </Menu.Content>
              </Menu.Root>
            </div>
          )}
        </Show>
      </div>
      </Show>

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
              onBrowsingChange={setBrowsing}
            />
          )}
        </Show>
      </div>
    </div>
  );
};

export default InstanceDetail;
