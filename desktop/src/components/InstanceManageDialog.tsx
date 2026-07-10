import { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Dialog } from "./Dialog";
import ServersPanel from "./ServersPanel";
import { RealmPanel } from "./RealmPanel";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { ModpackOverview } from "./ModpackOverview";
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { activeRoot, openInstance, isRunning, useAppStore } from "../store";
import { t, useLang } from "../i18n";
import type { InstanceConfig, InstanceSummary } from "../ipc/types";
import { TAB, TAB_ACTIVE, TABS } from "./instance-manage/shared";
import { ScreenshotsPanel } from "./instance-manage/ScreenshotsPanel";
import { PacksPanel } from "./instance-manage/PacksPanel";
import { WorldsPanel } from "./instance-manage/WorldsPanel";
import { SettingsTab } from "./instance-manage/SettingsTab";
import { ModsTab } from "./instance-manage/ModsTab";
import { useModsTab } from "./instance-manage/useModsTab";
import { useDropImport } from "./instance-manage/useDropImport";
import type { InstanceManageTab } from "./instance-manage/shared";

export type { InstanceManageTab } from "./instance-manage/shared";

/**
 * InstanceManageDialog —— 单实例管理:设置(名字/内存/Java/JVM/窗口)+ Mods(启停/删除)。
 * 设置改一项即 set_instance_config 持久化;Mods 用 set_mod_enabled / delete_mod。
 */

export function InstanceManageDialog(props: {
  open: boolean;
  instance: InstanceSummary | null;
  /** 关闭(仅非内嵌的 Dialog 模式使用;内嵌详情页无「完成」按钮)。 */
  onClose?: () => void;
  onChanged?: () => void;
  /** 复制完成回调,带新实例 id;调用方据此重拉列表并选中新实例。 */
  onCopied?: (newId: string) => void;
  /** 内嵌模式:不套 Dialog,直接铺在父容器里(实例详情页的设置标签用),
   *  隐藏实例名头部与「完成」按钮,父组件只在需要时挂载本组件即等于「打开」。 */
  embedded?: boolean;
  /** 受控 tab:启动页把实例管理页签提升到实例头部时使用。 */
  tab?: InstanceManageTab;
  onTabChange?: (tab: InstanceManageTab) => void;
  /** 隐藏本组件自带 tab 条,由外层渲染同级导航。 */
  hideTabs?: boolean;
  /** 进入/退出「添加」浏览模式(复用探索页占满内容区)时通知外层,详情页据此隐藏头部。 */
  onBrowsingChange?: (browsing: boolean) => void;
}) {
  useLang();
  const socialOn = useAppStore((s) => s.socialEnabled);
  const kobeSignedIn = useAppStore((s) => s.kobeUser !== null);

  const [internalTab, setInternalTab] = useState<InstanceManageTab>("settings");
  const [cfg, setCfg] = useState<InstanceConfig | null>(null);
  const [copying, setCopying] = useState(false);
  // 内存设置辅助:本机物理内存总量(MiB,一次即可)与按本实例 mod 数推荐的最大堆(MiB)。
  const [sysTotalMb, setSysTotalMb] = useState<number | null>(null);
  const [suggestedMb, setSuggestedMb] = useState<number | null>(null);
  // 「浏览/添加」模式:任一内容标签点「+ 添加」即进入,占满内容区(复用探索页)。
  const [browsing, setBrowsing] = useState(false);
  // keep-alive:记录已访问过的标签(惰性挂载——首访才挂,之后常驻、切走只以 display:none 隐藏)。
  const [visited, setVisited] = useState<Set<InstanceManageTab>>(new Set());

  const tab = props.tab ?? internalTab;
  const setTab = (next: InstanceManageTab) => {
    setInternalTab(next);
    props.onTabChange?.(next);
  };
  // 整合包来源(modrinth / curseforge)→ 多一个「概览」标签并置于首位。
  const modpackSource = (() => {
    const s = cfg?.source;
    return s && (s.provider === "modrinth" || s.provider === "curseforge") ? s : null;
  })();
  // 领域实例:多一个「领域」标签置于首位。仅在「社交开启 + 已登录 kobeMC」时把领域当作领域。
  const isRealm = !!props.instance?.realm && socialOn && kobeSignedIn;
  const visibleTabs = (): { key: InstanceManageTab; label: string }[] => {
    if (isRealm) {
      return [
        { key: "realm", label: t("instance.tabRealm") },
        ...(modpackSource ? [{ key: "overview" as const, label: t("instance.tabOverview") }] : []),
        { key: "mods", label: t("instance.tabMods") },
        { key: "resource_pack", label: t("instance.tabResourcePack") },
        { key: "shader", label: t("instance.tabShader") },
        { key: "datapack", label: t("instance.tabDatapack") },
        { key: "worlds", label: t("instance.tabWorlds") },
        { key: "servers", label: t("instance.tabServers") },
        { key: "settings", label: t("instance.tabSettings") },
        { key: "screenshots", label: t("instance.tabScreenshots") },
      ];
    }
    return modpackSource ? [{ key: "overview", label: t("instance.tabOverview") }, ...TABS()] : TABS();
  };

  // 是否「活动」(应加载数据 / 接受拖放):弹窗模式看 open,内嵌模式只要挂载即活动。
  const active = props.embedded || props.open;

  // 通知外层浏览态变化(详情页隐藏头部 + 本组件隐藏 tab 条)。
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => props.onBrowsingChange?.(browsing), [browsing]);

  // 首访即登记该标签(之后常驻);functional setState 免把 visited 放进 deps。
  useEffect(() => {
    if (active) setVisited((s) => (s.has(tab) ? s : new Set(s).add(tab)));
  }, [tab, active]);

  async function copyInstance() {
    const inst = props.instance;
    if (!inst) return;
    // 运行中复制会把正在写入的存档/文件拷成半截 —— 先要求停止。
    if (isRunning(inst.id)) {
      toast({ type: "error", message: t("instance.stopBeforeCopy") });
      return;
    }
    setCopying(true);
    try {
      const newId = await api.copyInstance(activeRoot(), inst.id, t("instance.copyName", { name: inst.name || inst.id }));
      toast({ type: "success", message: t("instance.copiedInstance") });
      props.onCopied?.(newId);
    } catch (e) {
      toast({ type: "error", message: t("instance.copyFailed", { err: String(e) }) });
    } finally {
      setCopying(false);
    }
  }

  // 系统内存只取一次;推荐值按本实例 mod 数计算,随实例变化。sysTotalMb 走 ref 读——否则本
  // effect 会订阅它自己 set 的 sysTotalMb,首次解析(null→值)重跑整个 effect、
  // 重复拉 getInstanceConfig/suggestInstanceMemory 并闪一下配置 spinner。
  const sysTotalRef = useRef<number | null>(null);

  // 打开/切换实例时拉配置 + 复位到设置页;关闭时清空。
  useEffect(() => {
    const inst = props.instance;
    m.setUpdates(null); // 切换实例/开关时清掉上一个实例的更新检查结果。
    if (active && inst) {
      setCfg(null);
      setSuggestedMb(null);
      api
        .getInstanceConfig(activeRoot(), inst.id)
        .then((c) => {
          setCfg(c);
          // 默认标签(仅非受控时):领域实例落「领域」,整合包来源落「概览」,其余落「设置」。
          if (props.tab === undefined)
            setInternalTab(
              isRealm ? "realm"
              : c.source?.provider === "modrinth" || c.source?.provider === "curseforge" ? "overview"
              : "settings",
            );
        })
        .catch((e) => toast({ type: "error", message: t("instance.readConfigFailed", { err: String(e) }) }));
      if (sysTotalRef.current === null)
        api
          .systemMemory()
          .then((m) => {
            sysTotalRef.current = m.total_mb;
            setSysTotalMb(m.total_mb);
          })
          .catch(() => {});
      api.suggestInstanceMemory(activeRoot(), inst.id).then(setSuggestedMb).catch(() => {});
    } else if (!active) {
      setCfg(null);
      setTab("settings");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.instance, active]);

  // Mods:仅在 Mods 标签 + 弹窗打开时拉取(gate:未满足条件不真正打后端)。
  const modsGated = active && !!props.instance && visited.has("mods");
  const m = useModsTab(props.instance, modsGated);
  const startBrowse = () => {
    m.setAddedMods(new Set<string>());
    setBrowsing(true);
  };
  // 切换标签(含外层受控切换)即退出浏览/添加模式并清掉详情;defer:首挂不跑(初值已是复位态)。
  const tabResetFirst = useRef(true);
  useEffect(() => {
    if (tabResetFirst.current) {
      tabResetFirst.current = false;
      return;
    }
    setBrowsing(false);
    m.setModDetail(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);

  // ---- 拖拽导入 ----
  const { dragOver, dropping, importTick, worldTick, dropAccepted } = useDropImport({
    instance: props.instance,
    active,
    tab,
    onModsImported: () => m.refetchMods(),
  });

  function patch(p: Partial<InstanceConfig>) {
    const cur = cfg;
    const inst = props.instance;
    if (!cur || !inst) return;
    const next = { ...cur, ...p };
    setCfg(next);
    void api
      .setInstanceConfig(activeRoot(), inst.id, next)
      .then(() => props.onChanged?.())
      .catch((e) => toast({ type: "error", message: t("instance.saveFailed", { err: String(e) }) }));
  }

  // 把内存滑块设为后端按系统内存 + mod 数推荐的值。
  function applyRecommendedMemory() {
    if (suggestedMb == null) return;
    patch({ memory_mb: suggestedMb });
  }

  async function pickIcon() {
    const inst = props.instance;
    if (!inst) return;
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: t("instance.imageFilter"), extensions: ["png", "jpg", "jpeg", "gif", "bmp", "webp"] }],
    });
    if (typeof picked !== "string") return; // 取消 / 多选(不会发生)
    try {
      await api.setInstanceIcon(activeRoot(), inst.id, picked);
      toast({ type: "success", message: t("instance.iconUpdated") });
      props.onChanged?.(); // 触发列表重拉,新图标随 list_instances 探测回来
    } catch (e) {
      toast({ type: "error", message: t("instance.setIconFailed", { err: String(e) }) });
    }
  }



  const body = (
    <div
      className={clsx("relative flex flex-col transition-shadow duration-150", {
        "max-h-[calc(100vh-100px)]": !props.embedded,
        "h-full": props.embedded,
        "ring-2 ring-inset ring-accent": dragOver,
      })}
    >
      {dragOver && dropAccepted && (
        <div className="absolute inset-0 z-10 grid place-items-center bg-window/85 pointer-events-none">
          <div className="text-[14px] text-accent font-semibold">{t("instance.dropToImport")}</div>
        </div>
      )}
      {dropping && (
        <div className="absolute inset-0 z-10 grid place-items-center bg-window/85">
          <div className="flex items-center gap-[10px] text-[14px] text-fg font-semibold">
            <Spinner size={18} /> {t("instance.importingOverlay")}
          </div>
        </div>
      )}
      {!props.embedded && (
        <Heading size="sub" className="px-[20px] pt-[18px]">
          {props.instance?.name || props.instance?.id}
        </Heading>
      )}

      {!props.hideTabs && !browsing && (
        <div className="shrink-0 flex gap-[4px] px-[16px] border-b border-titlebar mt-[10px] overflow-x-auto">
          {visibleTabs().map((item) => (
            <button
              key={item.key}
              className={`${TAB} whitespace-nowrap ${tab === item.key ? TAB_ACTIVE : ""}`}
              onClick={() => setTab(item.key)}
            >
              {item.label}
            </button>
          ))}
        </div>
      )}

      <div className="flex-1 min-h-0 p-[20px] flex flex-col gap-[14px] overflow-y-auto">
        {/* ---- 领域(同步 / 成员;领域实例的主标签)---- */}
        {visited.has("realm") && isRealm && props.instance && (
          <div className={clsx({ hidden: tab !== "realm" })}>
            <RealmPanel instance={props.instance} onChanged={() => props.onChanged?.()} />
          </div>
        )}

        {/* ---- 概览(整合包来源)---- */}
        {visited.has("overview") && modpackSource && (
          <div className={clsx({ hidden: tab !== "overview" })}>
            <ModpackOverview projectId={modpackSource.project_id} provider={modpackSource.provider} />
          </div>
        )}

        {/* ---- 设置 ---- */}
        {visited.has("settings") && (
          <div className={clsx("flex flex-col gap-[14px]", { hidden: tab !== "settings" })}>
            <SettingsTab
              instance={props.instance}
              cfg={cfg}
              setCfg={setCfg}
              patch={patch}
              pickIcon={pickIcon}
              sysTotalMb={sysTotalMb}
              suggestedMb={suggestedMb}
              applyRecommendedMemory={applyRecommendedMemory}
            />
          </div>
        )}

        {/* ---- Mods ---- */}
        {visited.has("mods") && (
          <div className={clsx("flex flex-col gap-[8px]", { hidden: tab !== "mods" })}>
            <ModsTab
              instance={props.instance}
              browsing={tab === "mods" && browsing}
              onExitBrowse={() => setBrowsing(false)}
              startBrowse={startBrowse}
              m={m}
              onLoaderAdded={(newId) => {
                props.onChanged?.();
                if (newId !== props.instance!.id) openInstance(newId);
              }}
            />
          </div>
        )}

        {/* ---- 资源包 / 光影 / 数据包 ---- */}
        {/* keep-alive:三个 pack 标签各自独立常驻,browse 只对当前激活的 pack 标签生效。 */}
        {visited.has("resource_pack") && props.instance && (
          <div className={clsx({ hidden: tab !== "resource_pack" })}>
            <PacksPanel
              instance={props.instance}
              kind="resource_pack"
              searchKind="resourcepack"
              emptyHint={t("instance.emptyResourcePack")}
              tick={importTick}
              browse={tab === "resource_pack" && browsing}
              onBrowse={setBrowsing}
            />
          </div>
        )}
        {visited.has("shader") && props.instance && (
          <div className={clsx({ hidden: tab !== "shader" })}>
            <PacksPanel
              instance={props.instance}
              kind="shader"
              searchKind="shader"
              emptyHint={t("instance.emptyShader")}
              tick={importTick}
              browse={tab === "shader" && browsing}
              onBrowse={setBrowsing}
            />
          </div>
        )}
        {visited.has("datapack") && props.instance && (
          <div className={clsx({ hidden: tab !== "datapack" })}>
            <PacksPanel
              instance={props.instance}
              kind="datapack"
              searchKind="datapack"
              emptyHint={t("instance.emptyDatapack")}
              tick={importTick}
              browse={tab === "datapack" && browsing}
              onBrowse={setBrowsing}
            />
          </div>
        )}

        {/* ---- 存档 ---- */}
        {visited.has("worlds") && props.instance && (
          <div className={clsx({ hidden: tab !== "worlds" })}>
            <WorldsPanel instance={props.instance} tick={worldTick} />
          </div>
        )}

        {/* ---- 多人服务器(servers.dat) ---- */}
        {visited.has("servers") && props.instance && (
          <div className={clsx("h-full min-h-0", { hidden: tab !== "servers" })}>
            <ServersPanel instance={props.instance} />
          </div>
        )}

        {/* ---- 截图 ---- */}
        {visited.has("screenshots") && props.instance && (
          <div className={clsx({ hidden: tab !== "screenshots" })}>
            <ScreenshotsPanel instance={props.instance} />
          </div>
        )}
      </div>

      {/* 内嵌模式(实例详情页)不渲染底部栏:复制实例移到详情页头部 ⋮ 菜单,完成本就不显示。 */}
      {!props.embedded && (
        <div className="flex justify-between items-center px-[20px] py-[14px] border-t border-titlebar">
          <Button variant="ghost" disabled={copying || !props.instance} onClick={copyInstance}>
            {copying ? t("instance.copying") : t("instance.copyInstance")}
          </Button>
          <Button variant="ghost" onClick={() => props.onClose?.()}>
            {t("instance.done")}
          </Button>
        </div>
      )}

      <Dialog
        open={m.confirmDelMod !== null}
        onClose={() => m.setConfirmDelMod(null)}
        label={t("instance.deleteMod")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <div className="text-[15px] font-semibold text-fg break-words">
            {t("instance.deleteModConfirm", { name: m.confirmDelMod?.name ?? "" })}
          </div>
          <div className="text-[13px] text-muted leading-[1.6]">{t("instance.deleteModBody")}</div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => m.setConfirmDelMod(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const mod = m.confirmDelMod;
                m.setConfirmDelMod(null);
                if (mod) void m.removeMod(mod);
              }}
            >
              {t("instance.delete")}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );

  // 内嵌模式直接铺在父容器;否则套 Dialog 作模态。
  return props.embedded ? (
    body
  ) : (
    <Dialog
      open={props.open}
      onClose={() => props.onClose?.()}
      label={t("instance.instanceManage")}
      contentClass="w-[520px] max-w-[calc(100vw-48px)] rounded-none overflow-hidden"
    >
      {body}
    </Dialog>
  );
}

export default InstanceManageDialog;
