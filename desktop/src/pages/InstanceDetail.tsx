import { Component, createResource, createSignal, onCleanup, onMount, Show } from "solid-js";
import { InstanceManageDialog, Dialog, toast, type InstanceRowData } from "../components";
import { PlayButton } from "../components/PlayButton";
import { Menu } from "../components/Menu";
import { formatRelativeTime } from "../components/format";
import { api, onInstallProgress } from "../ipc/api";
import { openInstanceDir, exportInstanceMrpack, deleteInstance } from "../util/instanceActions";
import { loaderLabel as fmtLoader } from "../util/loaders";
import { activeRoot, isRunning, isLaunching, playInstance, currentInstanceId, closeInstance, openInstance } from "../store";
import { t } from "../i18n";

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
  // 整合包更新检查(仅对由 Modrinth 整合包安装的实例返回非空);失败/无来源都安静返回空。
  const [updates, { refetch: refetchUpdates }] = createResource(
    () => currentInstanceId(),
    (id) => (id ? api.checkModpackUpdates(activeRoot(), id).catch(() => []) : []),
  );
  const [cfg] = createResource(
    () => currentInstanceId(),
    (id) => (id ? api.getInstanceConfig(activeRoot(), id).catch(() => null) : null),
  );
  const latestUpdate = () => (updates() ?? [])[0];
  const modrinthUrl = () => {
    const pid = cfg()?.source?.project_id;
    return pid ? `https://modrinth.com/project/${pid}` : null;
  };
  // 整合包就地更新:确认弹窗 + 进度。覆盖导入新包到既有实例,存档/配置保留,被移除的模组进回收站。
  const [updateOpen, setUpdateOpen] = createSignal(false);
  const [updating, setUpdating] = createSignal(false);
  const [updateProgress, setUpdateProgress] = createSignal("");

  async function applyUpdate() {
    const i = inst();
    const target = latestUpdate();
    if (!i || !target) return;
    setUpdating(true);
    setUpdateProgress("");
    const off = onInstallProgress((p) =>
      setUpdateProgress(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage),
    );
    try {
      const out = await api.applyModpackUpdate(activeRoot(), i.id, target.id);
      toast({ type: "success", message: t("instance.updateSuccess", { version: target.version_number }) });
      if (out.removed.length > 0)
        toast({ type: "info", message: t("instance.updateRemoved", { count: out.removed.length }) });
      setUpdateOpen(false);
      void refetch();
      void refetchUpdates();
    } catch (e) {
      toast({ type: "error", message: t("instance.updateFailed", { err: String(e) }) });
    } finally {
      off();
      setUpdating(false);
      setUpdateProgress("");
    }
  }
  // 进入「添加」浏览模式时整页让给复用的探索视图,隐藏头部(返回路径用视图内的「← 返回已安装」)。
  const [browsing, setBrowsing] = createSignal(false);
  // 删除实例前确认(与实例行的删除确认一致,避免 ⋮ 菜单一点就删)。
  const [confirmDel, setConfirmDel] = createSignal(false);

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


  const loaderLabel = () => {
    const i = inst();
    if (!i) return "";
    const name = fmtLoader(i.loader);
    return name ? `${name} ${i.mc_version}` : i.mc_version;
  };
  const playedLabel = () => {
    const i = inst();
    if (!i) return "";
    const rel = formatRelativeTime(i.last_played ?? 0);
    return rel === "never" ? t("instance.neverPlayed") : t("instance.lastPlayed", { rel });
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
    if (isRunning(i.id)) {
      toast({ type: "error", message: t("instance.stopBeforeCopyDetail") });
      return;
    }
    try {
      const newId = await api.copyInstance(activeRoot(), i.id, t("instance.copyName", { name: i.name || i.id }));
      toast({ type: "success", message: t("instance.copiedInstance") });
      openInstance(newId);
    } catch (e) {
      toast({ type: "error", message: t("instance.copyFailed", { err: String(e) }) });
    }
  }

  async function onMenuAction(value: string) {
    const i = inst();
    const row = toRowData();
    if (!i || !row) return;
    if (value === "open") void openInstanceDir(activeRoot(), i.id);
    else if (value === "copy") await copyCurrent();
    else if (value === "export") void exportInstanceMrpack(activeRoot(), row);
    else if (value === "delete") setConfirmDel(true);
  }

  async function doDelete() {
    const i = inst();
    if (!i) return;
    setConfirmDel(false);
    if (await deleteInstance(activeRoot(), { id: i.id, name: i.name || i.id })) closeInstance();
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
          aria-label={t("instance.back")}
        >
          <svg class="w-[16px] h-[16px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="m14 6-6 6 6 6" />
          </svg>
          {t("instance.back")}
        </button>

        <Show
          when={inst()}
          fallback={<div class="text-dim text-[14px] py-[8px]">{t("instance.loading")}</div>}
        >
          {(i) => (
            <div class="flex items-center gap-[14px]">
              <div class="relative shrink-0 w-[52px] h-[52px] rounded-ctl overflow-hidden grid place-items-center bg-gradient-to-br from-a-3 to-a-5 text-white font-bold text-[22px] uppercase select-none">
                <Show when={i().icon} fallback={<span>{(i().name || i().id).charAt(0)}</span>}>
                  <img src={i().icon!} alt="" width="52" height="52" class="w-full h-full object-cover" />
                </Show>
                <Show when={isRunning(i().id)}>
                  <span class="absolute right-[3px] bottom-[3px] w-[12px] h-[12px] rounded-full bg-a-5 shadow-[0_0_0_2px_var(--bg-card)]" title={t("instance.running")} />
                </Show>
              </div>
              <div class="flex-1 min-w-0">
                <div class="text-[20px] font-bold text-fg whitespace-nowrap overflow-hidden text-ellipsis" title={i().name || i().id}>
                  {i().name || i().id}
                </div>
                <div class="text-[12px] text-dim whitespace-nowrap overflow-hidden text-ellipsis">
                  {loaderLabel()} · {playedLabel()}
                </div>
                {/* 整合包更新提示:有更新时给个可点击的小药丸,点开确认弹窗就地更新。 */}
                <Show when={latestUpdate()}>
                  <button
                    type="button"
                    class="inline-flex items-center gap-[5px] mt-[5px] h-[22px] pl-[8px] pr-[9px] rounded-full bg-a-1 border border-a-4/40 text-a-7 text-[11px] font-semibold no-underline cursor-pointer transition-colors duration-150 hover:bg-a-2"
                    title={t("instance.updateAvailableHint")}
                    onClick={() => setUpdateOpen(true)}
                  >
                    <span class="w-[6px] h-[6px] rounded-full bg-a-5 shrink-0" aria-hidden="true" />
                    {t("instance.updateAvailable", { version: latestUpdate()!.version_number })}
                  </button>
                </Show>
              </div>
              <PlayButton running={isRunning(i().id)} disabled={isLaunching(i().id)} onClick={() => void playInstance(i().id)} />
              <Menu.Root positioning={{ placement: "bottom-end" }} onSelect={(d: { value: string }) => void onMenuAction(d.value)}>
                <Menu.Trigger
                  class="inline-flex items-center justify-center w-[34px] h-[34px] border-none bg-transparent text-dim rounded-ctl cursor-pointer transition-[background-color,color] duration-[var(--dur)] ease-app hover:bg-glass-hover hover:text-fg data-[state=open]:bg-glass-hover data-[state=open]:text-fg"
                  aria-label={t("instance.moreActions")}
                >
                  <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
                    <circle cx="8" cy="3" r="1.5" />
                    <circle cx="8" cy="8" r="1.5" />
                    <circle cx="8" cy="13" r="1.5" />
                  </svg>
                </Menu.Trigger>
                <Menu.Content>
                  <Menu.Item value="open">{t("instance.openGameDir")}</Menu.Item>
                  <Menu.Item value="copy">{t("instance.copyInstanceItem")}</Menu.Item>
                  <Menu.Item value="export">{t("instance.exportModpack")}</Menu.Item>
                  <Menu.Separator />
                  <Menu.Item value="delete" danger>
                    {t("instance.deleteInstance")}
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

      <Dialog
        open={confirmDel()}
        onClose={() => setConfirmDel(false)}
        label={t("instance.deleteInstance")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-fg break-words">
            {t("instance.deleteInstanceConfirm", { name: inst()?.name || inst()?.id || "" })}
          </div>
          <div class="text-[13px] text-dim leading-[1.6]">
            {t("instance.deleteInstanceBodyDetail")}
          </div>
          <div class="flex justify-end gap-[10px]">
            <button
              class="h-[34px] px-[16px] border border-glass-border rounded-ctl bg-glass-card text-fg text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-glass-hover"
              onClick={() => setConfirmDel(false)}
            >
              {t("instance.cancel")}
            </button>
            <button
              class="h-[34px] px-[16px] border-none rounded-ctl bg-danger text-white text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-danger-hover"
              onClick={() => void doDelete()}
            >
              {t("instance.delete")}
            </button>
          </div>
        </div>
      </Dialog>

      <Dialog
        open={updateOpen()}
        onClose={() => !updating() && setUpdateOpen(false)}
        label={t("instance.updateTitle")}
        contentClass="w-[400px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden"
      >
        <div class="p-[20px] flex flex-col gap-[14px]">
          <div class="text-[15px] font-semibold text-fg break-words">{t("instance.updateTitle")}</div>
          <div class="text-[13px] text-dim leading-[1.6]">
            {t("instance.updateBody", { version: latestUpdate()?.version_number ?? "" })}
          </div>
          <Show when={modrinthUrl()}>
            <a
              href={modrinthUrl()!}
              class="self-start text-[12px] text-a-7 no-underline hover:underline"
            >
              {t("instance.viewOnModrinth")} →
            </a>
          </Show>
          <Show when={updating() && updateProgress()}>
            <div class="text-[12px] text-dim font-mono truncate">{updateProgress()}</div>
          </Show>
          <div class="flex justify-end gap-[10px]">
            <button
              disabled={updating()}
              class="h-[34px] px-[16px] border border-glass-border rounded-ctl bg-glass-card text-fg text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-glass-hover disabled:opacity-50 disabled:cursor-not-allowed"
              onClick={() => setUpdateOpen(false)}
            >
              {t("instance.cancel")}
            </button>
            <button
              disabled={updating()}
              class="h-[34px] px-[16px] border-none rounded-ctl bg-a-5 text-white text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-a-6 disabled:opacity-60 disabled:cursor-not-allowed"
              onClick={() => void applyUpdate()}
            >
              {updating()
                ? t("instance.updating")
                : t("instance.updateNow", { version: latestUpdate()?.version_number ?? "" })}
            </button>
          </div>
        </div>
      </Dialog>
    </div>
  );
};

export default InstanceDetail;
