import { Component, createEffect, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { Avatar, BlockedFilesDialog, InstanceManageDialog, NewInstanceDialog, Spinner, toast } from "../components";
import { api, onGameLog, onLaunchProgress } from "../ipc/api";
import { activeRoot, isRunning } from "../store";
import { openInstanceDir } from "../util/instanceActions";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { AccountSummary, ImportOutcome, InstanceSummary } from "../ipc/types";
import type { InstanceManageTab } from "../components/InstanceManageDialog";
import ClassicAccountDialog from "./ClassicAccountDialog";
import "./ClassicLaunch.css";

const loaderLabel = (l: string) =>
  ({ vanilla: "原版", forge: "Forge", neoforge: "NeoForge", fabric: "Fabric", quilt: "Quilt" } as Record<string, string>)[l] ?? l;

const kindLabel = (k: string) =>
  k === "microsoft" ? "正版验证" : k === "yggdrasil" ? "外置登录" : "离线模式";

/** 右侧主区视图:新闻主页 / 版本选择 / 实例详情 / 启动日志。 */
type RightView = "news" | "versions" | "instance" | "log";
/** 实例详情内的子标签:首页(概览)+ 实例管理各页,资源子类不再藏到二级 tab。 */
type InstanceTab = "home" | InstanceManageTab;

const INSTANCE_TABS: { key: InstanceTab; label: string }[] = [
  { key: "home", label: "首页" },
  { key: "settings", label: "设置" },
  { key: "mods", label: "Mods" },
  { key: "resource_pack", label: "资源包" },
  { key: "shader", label: "光影" },
  { key: "datapack", label: "数据包" },
  { key: "worlds", label: "存档" },
  { key: "screenshots", label: "截图" },
];

/**
 * ClassicLaunch —— 经典启动页:
 *   - 左栏:账号卡(皮肤头像 + 用户名 + 验证方式)+ 招牌「启动游戏」按钮
 *     (副标题=当前版本)+「版本选择 / 版本设置」两个并排按钮。
 *   - 右栏:默认是「新闻主页」(欢迎卡 + 折叠卡 + 最新快照);点「版本选择」
 *     切到版本列表;启动后切到实时日志。版本列表不再常驻铺在启动页。
 */
const ClassicLaunch: Component = () => {
  const [instances, { refetch: refetchInstances }] = createResource(
    () => activeRoot(),
    (root) => api.listInstances(root),
  );
  const [accounts, { refetch: refetchAccounts }] = createResource(() => api.listAccounts());
  // 新闻主页用:最新快照(含 snapshot),只取头一个做展示。
  const [snapshots] = createResource(() => api.listVersions(true));

  const [selected, setSelected] = createSignal<InstanceSummary | null>(null);
  const [logs, setLogs] = createSignal<string[]>([]);
  const [launching, setLaunching] = createSignal(false);
  const [rightView, setRightView] = createSignal<RightView>("news");
  const [importOutcome, setImportOutcome] = createSignal<ImportOutcome | null>(null);
  const [instanceTab, setInstanceTab] = createSignal<InstanceTab>("home");
  const [newsOpen, setNewsOpen] = createSignal<Record<string, boolean>>({ snapshot: true });
  const [showLogin, setShowLogin] = createSignal(false);
  const [showNew, setShowNew] = createSignal(false);
  const activeManageTab = (): InstanceManageTab =>
    instanceTab() === "home" ? "settings" : (instanceTab() as InstanceManageTab);

  // 打开某个实例的详情视图(右栏),默认落在「首页」标签。
  function openInstance(inst: InstanceSummary) {
    setSelected(inst);
    setInstanceTab("home");
    setRightView("instance");
  }
  // 整合包导入/导出的进行态(禁用按钮 + 文案)。
  const [busy, setBusy] = createSignal<"" | "import" | "export">("");

  // 默认选中第一个版本(供启动按钮的副标题与启动用)。
  createEffect(() => {
    const list = instances();
    if (!selected() && list && list.length > 0) setSelected(list[0]);
  });

  const recentInstances = () => (instances() ?? []).slice(0, 4);

  // 当前账号:优先 selected,否则第一个。
  const activeAccount = (): AccountSummary | undefined => {
    const list = accounts();
    if (!list || list.length === 0) return undefined;
    return list.find((a) => a.selected) ?? list[0];
  };

  // 最新快照(版本清单第一个 kind=snapshot;退而求其次取第一个)。
  const latestSnapshot = () => {
    const list = snapshots();
    if (!list || list.length === 0) return undefined;
    return list.find((v) => v.kind === "snapshot") ?? list[0];
  };

  const offLog = onGameLog((l) => setLogs((p) => [...p.slice(-400), l.line]));
  const offProg = onLaunchProgress((p) => p.stage && setLogs((s) => [...s.slice(-400), `· ${p.stage}`]));
  onCleanup(() => {
    offLog();
    offProg();
  });

  // 选中实例是否在运行(运行态由 store 依 game://started/exit 维护)。
  const selectedRunning = () => {
    const inst = selected();
    return !!inst && isRunning(inst.id);
  };

  async function launch() {
    const inst = selected();
    if (!inst) {
      toast({ type: "error", message: "请先在「版本选择」里选一个版本" });
      setRightView("versions");
      return;
    }
    // 运行中再点 = 停止。
    if (isRunning(inst.id)) {
      try {
        await api.stopInstance(inst.id);
      } catch (e) {
        toast({ type: "error", message: `停止失败:${e}` });
      }
      return;
    }
    const acc = activeAccount();
    const name = acc?.username ?? "Player";
    const online = !!acc && acc.kind !== "offline";
    setLaunching(true);
    setRightView("log");
    setLogs([`正在启动 ${inst.name || inst.id} …`]);
    try {
      await api.launchInstance(activeRoot(), inst.id, name, online);
      // 成功/退出/崩溃的 toast 由 store 统一发(基于真实进程事件)。
    } catch (e) {
      toast({ type: "error", message: `启动失败:${e}` });
      setLogs((p) => [...p, `启动失败:${e}`]);
    } finally {
      setLaunching(false);
    }
  }

  // 导入整合包:选文件(.mrpack/.zip,自动识别格式)→ 建实例 → 刷新版本列表。
  async function importModpack() {
    if (busy()) return;
    const picked = await openDialog({
      title: "选择整合包",
      multiple: false,
      filters: [{ name: "整合包", extensions: ["mrpack", "zip"] }],
    });
    if (!picked || typeof picked !== "string") return;
    setBusy("import");
    try {
      const out = await api.importModpack(activeRoot(), picked, null);
      if (out.blocked.length > 0 || out.skipped_optional.length > 0) {
        setImportOutcome(out); // 弹窗摊开需手动下载 / 被跳过的文件
      } else {
        toast({ type: "success", message: `已导入整合包「${out.instance_id}」` });
      }
      await refetchInstances();
      setRightView("versions");
    } catch (e) {
      toast({ type: "error", message: `导入失败:${e}` });
    } finally {
      setBusy("");
    }
  }

  // 导出当前选中版本为 .mrpack(本地未能反查到的文件落入 overrides/)。
  async function exportSelected() {
    if (busy()) return;
    const inst = selected();
    if (!inst) {
      toast({ type: "error", message: "请先在「版本选择」里选一个版本再导出" });
      setRightView("versions");
      return;
    }
    const dest = await saveDialog({
      title: "导出整合包",
      defaultPath: `${inst.name || inst.id}.mrpack`,
      filters: [{ name: "Modrinth 整合包", extensions: ["mrpack"] }],
    });
    if (!dest) return;
    setBusy("export");
    try {
      const out = await api.exportModpack({
        root: activeRoot(),
        instanceId: inst.id,
        target: "modrinth",
        dest,
        packName: inst.name || inst.id,
        mcVersion: inst.mc_version,
        loader: inst.loader,
        loaderVersion: inst.loader_version || null,
      });
      toast({ type: "success", message: `已导出:${out}` });
    } catch (e) {
      toast({ type: "error", message: `导出失败:${e}` });
    } finally {
      setBusy("");
    }
  }

  return (
    <div class="grid grid-cols-[300px_1fr] h-full min-h-0 bg-transparent">
      {/* ===== 左栏:账号卡 + 启动区(Classic 固定窄栏) ===== */}
      <aside class="flex min-h-0 flex-col glass-panel border-r border-glass-divider">
        {/* 账号卡:皮肤头像 + 用户名 + 验证方式,点击打开登录/切换弹窗 */}
        <button
          class="flex w-full items-center gap-[12px] border-none bg-transparent px-[18px] py-[16px] text-left cursor-pointer transition-[background] duration-150 ease-[ease] hover:bg-classic-blue-lightest"
          onClick={() => setShowLogin(true)}
          title="点击登录 / 切换账号"
        >
          <Show
            when={!accounts.loading}
            fallback={
              <div class="w-[58px] h-[58px] flex-[0_0_58px] rounded-[10px] flex items-center justify-center text-[24px] font-extrabold text-white shadow-classic [image-rendering:pixelated] bg-classic-blue-bg2 animate-[classic-pulse_1.3s_ease-in-out_infinite]" />
            }
          >
            <div class="w-[58px] h-[58px] flex-[0_0_58px] rounded-[10px] flex items-center justify-center text-[24px] font-extrabold text-white shadow-classic [image-rendering:pixelated] bg-[linear-gradient(135deg,var(--classic-blue-hover),var(--classic-blue))]">
              <Avatar kind={activeAccount()?.kind} uuid={activeAccount()?.uuid} />
            </div>
            <div class="min-w-0">
              <div class="text-[16px] font-bold text-classic-text max-w-full whitespace-nowrap overflow-hidden text-ellipsis">
                {activeAccount()?.username ?? "未登录"}
              </div>
              <div class="text-[12px] text-classic-text3 mt-[3px]">
                {activeAccount() ? kindLabel(activeAccount()!.kind) : "点击登录账号"}
              </div>
            </div>
          </Show>
        </button>

        <div class="border-t border-glass-divider px-[18px] py-[14px]">
          <div class="text-[12px] font-semibold text-classic-text3 mb-[8px]">当前版本</div>
          <Show
            when={selected()}
            fallback={<div class="text-[13px] text-classic-text3 leading-[1.6]">还没有选择版本</div>}
          >
            <div class="flex items-center gap-[10px] min-w-0">
              <span
                class="w-[34px] h-[34px] flex-[0_0_34px] rounded-[4px] flex items-center justify-center font-bold text-[14px] text-white bg-classic-blue data-[loader=forge]:bg-[#c96a1c] data-[loader=neoforge]:bg-[#c96a1c] data-[loader=fabric]:bg-[#a87b3f] data-[loader=quilt]:bg-[#a87b3f]"
                data-loader={selected()!.loader}
              >
                {(selected()!.name || selected()!.id)[0]?.toUpperCase()}
              </span>
              <span class="flex flex-col min-w-0">
                <span class="text-[13px] font-semibold text-classic-text whitespace-nowrap overflow-hidden text-ellipsis">
                  {selected()!.name || selected()!.id}
                </span>
                <span class="text-[11px] text-classic-text3">
                  {selected()!.mc_version} · {loaderLabel(selected()!.loader)}
                </span>
              </span>
            </div>
          </Show>
        </div>

        {/* 启动区:招牌按钮(描边 + 版本副标题)+ 版本选择/新建实例 */}
        <div class="px-[18px] pb-[14px]">
          <button
            class="w-full h-[54px] flex flex-col items-center justify-center gap-px border-[1.5px] border-classic-blue rounded-[3px] bg-classic-blue-lightest cursor-pointer transition-[background,box-shadow,transform] duration-150 ease-[ease] enabled:hover:bg-classic-blue-bg enabled:hover:shadow-[0_3px_10px_rgba(19,112,243,0.2)] enabled:active:scale-[0.99] disabled:opacity-[0.55] disabled:cursor-not-allowed"
            disabled={launching()}
            onClick={launch}
          >
            <span class="text-[16px] font-bold tracking-[1px] text-classic-blue">
              {launching() ? "启动中…" : selectedRunning() ? "停止游戏" : "启动游戏"}
            </span>
            <span class="text-[11px] text-classic-text3 max-w-full whitespace-nowrap overflow-hidden text-ellipsis">
              <Show when={selected()} fallback={"未选择版本"}>
                {selected()!.name || selected()!.id}
              </Show>
            </span>
          </button>
          <div class="mt-[10px]">
            <button
              class="w-full h-[36px] border border-glass-border rounded-[3px] bg-glass-card text-classic-text text-[13px] cursor-pointer transition-[background,border-color,color] duration-150 ease-[ease] hover:bg-classic-blue-lightest hover:border-classic-blue-soft"
              classList={{
                "bg-classic-blue-bg": rightView() === "versions" || rightView() === "instance",
                "border-classic-blue": rightView() === "versions" || rightView() === "instance",
                "text-classic-blue": rightView() === "versions" || rightView() === "instance",
                "font-semibold": rightView() === "versions" || rightView() === "instance",
              }}
              onClick={() => setRightView(rightView() === "versions" ? "news" : "versions")}
            >
              版本选择
            </button>
          </div>
          <button
            class="w-full h-[36px] mt-[10px] border border-classic-blue rounded-[3px] bg-classic-blue-lightest text-classic-blue text-[13px] font-semibold cursor-pointer transition-[background] duration-150 ease-[ease] hover:bg-classic-blue-bg"
            onClick={() => setShowNew(true)}
          >
            + 新建实例
          </button>
        </div>

        <div class="min-h-0 flex-1 border-t border-glass-divider px-[18px] py-[14px] overflow-y-auto">
          <div class="flex items-center justify-between mb-[10px]">
            <span class="text-[12px] font-semibold text-classic-text3">最近版本</span>
            <button
              class="border-none bg-transparent p-0 text-[12px] text-classic-blue cursor-pointer hover:underline"
              onClick={() => setRightView("versions")}
            >
              全部
            </button>
          </div>
          <Show when={!instances.loading} fallback={<div class="flex justify-center p-[12px]"><Spinner /></div>}>
            <Show
              when={recentInstances().length > 0}
              fallback={
                <div class="rounded-[5px] border border-dashed border-glass-border bg-glass-card px-[12px] py-[14px] text-[12px] leading-[1.7] text-classic-text3">
                  还没有版本。去「下载」安装，或新建一个实例。
                </div>
              }
            >
              <div class="flex flex-col gap-[6px]">
                <For each={recentInstances()}>
                  {(inst) => (
                    <button
                      class="flex items-center gap-[8px] w-full border border-transparent rounded-[4px] bg-transparent px-[8px] py-[7px] text-left cursor-pointer transition-[background,border-color] duration-150 hover:bg-classic-blue-lightest hover:border-classic-blue-soft"
                      classList={{
                        "bg-classic-blue-bg border-classic-blue-soft": selected()?.id === inst.id,
                      }}
                      onClick={() => openInstance(inst)}
                    >
                      <span
                        class="w-[26px] h-[26px] flex-[0_0_26px] rounded-[4px] flex items-center justify-center font-bold text-[12px] text-white bg-classic-blue data-[loader=forge]:bg-[#c96a1c] data-[loader=neoforge]:bg-[#c96a1c] data-[loader=fabric]:bg-[#a87b3f] data-[loader=quilt]:bg-[#a87b3f]"
                        data-loader={inst.loader}
                      >
                        {(inst.name || inst.id)[0]?.toUpperCase()}
                      </span>
                      <span class="flex flex-col min-w-0">
                        <span class="text-[12px] font-semibold text-classic-text whitespace-nowrap overflow-hidden text-ellipsis">
                          {inst.name || inst.id}
                        </span>
                        <span class="text-[11px] text-classic-text3">
                          {inst.mc_version} · {loaderLabel(inst.loader)}
                        </span>
                      </span>
                    </button>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </div>
      </aside>

      {/* ===== 右栏:新闻主页 / 版本选择 / 启动日志 ===== */}
      <section class="flex flex-col min-h-0 py-[18px] px-[22px] gap-[12px] overflow-auto">
        {/* --- 新闻主页(默认) --- */}
        <Show when={rightView() === "news"}>
          <div class="flex flex-col gap-[12px]">
            <div class="glass-card rounded-[5px] py-[14px] px-[16px] flex items-center gap-[12px] bg-[linear-gradient(90deg,var(--classic-blue-lightest),var(--classic-card))] border-l-[3px] border-classic-blue">
              <span class="text-[22px]">📰</span>
              <div>
                <div class="text-[14px] font-bold text-classic-text">欢迎使用 MC Launcher</div>
                <div class="text-[12px] text-classic-text3 mt-[3px]">左侧选择账号与版本,点「启动游戏」即可开玩。</div>
              </div>
            </div>

            <button
              class="glass-card glass-card--hover rounded-[5px] py-[14px] px-[16px] flex items-center justify-between w-full border-none cursor-pointer text-left transition-[box-shadow] duration-150 ease-[ease]"
              onClick={() => setNewsOpen((o) => ({ ...o, cape: !o.cape }))}
            >
              <span class="text-[14px] font-bold text-classic-text">新披风与披风迁移</span>
              <span
                class="text-classic-text3 text-[16px] transition-transform duration-200 ease-[ease]"
                classList={{ "rotate-180": !!newsOpen().cape }}
              >
                ⌄
              </span>
            </button>
            <Show when={newsOpen().cape}>
              <div class="glass-card rounded-[5px] py-[14px] px-[16px] text-[13px] text-classic-text2">
                <p class="m-0 leading-[1.7]">Mojang 已开放披风迁移。绑定正版账号后可在游戏内更换披风。</p>
              </div>
            </Show>

            <button
              class="glass-card glass-card--hover rounded-[5px] py-[14px] px-[16px] flex items-center justify-between w-full border-none cursor-pointer text-left transition-[box-shadow] duration-150 ease-[ease]"
              onClick={() => setNewsOpen((o) => ({ ...o, snapshot: !o.snapshot }))}
            >
              <span class="text-[14px] font-bold text-classic-text">
                最新快照版
                <Show when={latestSnapshot()}>{` - ${latestSnapshot()!.id}`}</Show>
              </span>
              <span
                class="text-classic-text3 text-[16px] transition-transform duration-200 ease-[ease]"
                classList={{ "rotate-180": !!newsOpen().snapshot }}
              >
                ⌄
              </span>
            </button>
            <Show when={newsOpen().snapshot}>
              <div class="glass-card rounded-[5px] py-[14px] px-[16px] text-[13px] text-classic-text2">
                <div class="h-[150px] rounded-[5px] bg-[linear-gradient(135deg,#7fb0f7_0%,#4890f5_45%,#1370f3_100%)] flex items-center justify-center">
                  <Show when={latestSnapshot()} fallback={<Spinner />}>
                    <span class="py-[8px] px-[22px] rounded-[6px] bg-[rgba(11,91,203,0.85)] text-white text-[22px] font-bold tracking-[1px]">{latestSnapshot()!.id}</span>
                  </Show>
                </div>
                <div class="text-[14px] font-bold mt-[12px] text-classic-blue">新特性</div>
                <Show when={latestSnapshot()}>
                  <ul class="list-disc mt-[6px] mb-0 pl-[18px] text-classic-text2 text-[13px] leading-[1.9]">
                    <li>发布时间:{new Date(latestSnapshot()!.release_time).toLocaleDateString()}</li>
                    <li>到「下载」标签页可一键安装该快照</li>
                    <li>快照为开发版,建议另建实例体验</li>
                  </ul>
                </Show>
              </div>
            </Show>
          </div>
        </Show>

        {/* --- 版本选择(点「版本选择」进入) --- */}
        <Show when={rightView() === "versions"}>
          <div class="glass-card rounded-[5px] flex flex-col min-h-0 overflow-hidden flex-1">
            <div class="flex items-center justify-between gap-[8px] py-[10px] px-[16px] text-[13px] font-bold text-classic-text border-b border-glass-divider">
              <span>版本选择</span>
              <span class="flex gap-[6px]">
                <button
                  class="border border-classic-blue text-classic-blue bg-transparent rounded-[3px] py-[4px] px-[10px] text-[12px] font-semibold cursor-pointer transition-[background,color] duration-[var(--mo-dur-fast,150ms)] ease-[var(--mo-ease-out,ease)] enabled:hover:bg-classic-blue enabled:hover:text-white disabled:opacity-50 disabled:cursor-default"
                  disabled={busy() !== ""}
                  onClick={importModpack}
                >
                  {busy() === "import" ? "导入中…" : "导入整合包"}
                </button>
                <button
                  class="border border-classic-blue text-classic-blue bg-transparent rounded-[3px] py-[4px] px-[10px] text-[12px] font-semibold cursor-pointer transition-[background,color] duration-[var(--mo-dur-fast,150ms)] ease-[var(--mo-ease-out,ease)] enabled:hover:bg-classic-blue enabled:hover:text-white disabled:opacity-50 disabled:cursor-default"
                  disabled={busy() !== "" || !selected()}
                  onClick={exportSelected}
                >
                  {busy() === "export" ? "导出中…" : "导出整合包"}
                </button>
              </span>
            </div>
            <div class="overflow-y-auto py-[4px]">
              <Show when={!instances.loading} fallback={<div class="flex justify-center p-[28px]"><Spinner /></div>}>
                <Show
                  when={(instances() ?? []).length > 0}
                  fallback={<div class="py-[28px] px-[16px] text-classic-text3 text-[13px] text-center leading-[1.9]">还没有版本<br />去「下载」装一个</div>}
                >
                  <For each={instances() ?? []}>
                    {(inst) => (
                      <button
                        class="relative flex items-center gap-[10px] w-full h-[46px] pl-[16px] pr-[18px] border-none bg-transparent cursor-pointer text-left transition-[background] duration-150 ease-[ease] hover:bg-classic-blue-bg2 before:content-[''] before:absolute before:left-0 before:top-[7px] before:bottom-[7px] before:w-[3px] before:rounded-[0_2px_2px_0] before:bg-transparent before:transition-[background] before:duration-150 before:ease-[ease] hover:before:bg-classic-blue-soft"
                        classList={{
                          "bg-classic-blue-bg": selected()?.id === inst.id,
                          "before:bg-classic-blue": selected()?.id === inst.id,
                        }}
                        onClick={() => openInstance(inst)}
                      >
                        <span
                          class="w-[30px] h-[30px] flex-[0_0_30px] rounded-[4px] flex items-center justify-center font-bold text-[14px] text-white bg-classic-blue data-[loader=forge]:bg-[#c96a1c] data-[loader=neoforge]:bg-[#c96a1c] data-[loader=fabric]:bg-[#a87b3f] data-[loader=quilt]:bg-[#a87b3f]"
                          data-loader={inst.loader}
                        >
                          {(inst.name || inst.id)[0]?.toUpperCase()}
                        </span>
                        <span class="flex flex-col min-w-0">
                          <span class="text-classic-text text-[13px] whitespace-nowrap overflow-hidden text-ellipsis">{inst.name || inst.id}</span>
                          <span class="text-classic-text3 text-[11px]">{inst.mc_version} · {loaderLabel(inst.loader)}</span>
                        </span>
                      </button>
                    )}
                  </For>
                </Show>
              </Show>
            </div>
          </div>
        </Show>

        {/* --- 实例详情:首页(概览)+ 实例管理各页 --- */}
        <Show when={rightView() === "instance" && selected()}>
          {(inst) => (
            <div class="flex flex-col min-h-0 flex-1 gap-[12px]">
              {/* 头部:图标 + 名称 + 版本信息 + 同级实例标签 */}
              <div class="glass-card rounded-[5px] overflow-hidden">
                <div class="flex items-center gap-[14px] p-[16px]">
                  <div class="w-[52px] h-[52px] rounded-[8px] overflow-hidden bg-classic-blue-bg2 grid place-items-center shrink-0">
                    <Show
                      when={inst().icon}
                      fallback={
                        <span class="text-[24px] font-extrabold text-classic-blue">
                          {(inst().name || inst().id)[0]?.toUpperCase()}
                        </span>
                      }
                    >
                      <img src={inst().icon!} alt="" width="52" height="52" class="w-full h-full object-cover" />
                    </Show>
                  </div>
                  <div class="min-w-0">
                    <div class="text-[18px] font-bold text-classic-text whitespace-nowrap overflow-hidden text-ellipsis">
                      {inst().name || inst().id}
                    </div>
                    <div class="text-[12px] text-classic-text3">
                      Minecraft {inst().mc_version} · {loaderLabel(inst().loader)}
                      <Show when={inst().loader_version}>{` ${inst().loader_version}`}</Show>
                    </div>
                  </div>
                </div>
                <div class="flex gap-[4px] px-[12px] border-t border-glass-divider overflow-x-auto">
                  <For each={INSTANCE_TABS}>
                    {(item) => (
                      <button
                        class="px-[16px] py-[9px] text-[13px] font-semibold cursor-pointer border-b-2 border-b-transparent text-classic-text3 hover:text-classic-text transition-colors duration-150 whitespace-nowrap"
                        classList={{ "!text-classic-blue !border-b-classic-blue": instanceTab() === item.key }}
                        onClick={() => setInstanceTab(item.key)}
                      >
                        {item.label}
                      </button>
                    )}
                  </For>
                </div>
              </div>

              {/* 首页:概览 + 快捷操作 */}
              <Show when={instanceTab() === "home"}>
                <div class="flex flex-col gap-[12px]">
                  <div class="glass-card rounded-[5px] py-[14px] px-[16px]">
                    <div class="text-[13px] font-bold text-classic-text mb-[8px]">游玩信息</div>
                    <div class="grid grid-cols-2 gap-y-[6px] text-[13px]">
                      <span class="text-classic-text3">游戏版本</span>
                      <span class="text-classic-text2">{inst().mc_version}</span>
                      <span class="text-classic-text3">加载器</span>
                      <span class="text-classic-text2">
                        {loaderLabel(inst().loader)}
                        <Show when={inst().loader_version}>{` ${inst().loader_version}`}</Show>
                      </span>
                      <span class="text-classic-text3">上次游玩</span>
                      <span class="text-classic-text2">
                        {inst().last_played
                          ? new Date(inst().last_played!).toLocaleString()
                          : "从未"}
                      </span>
                    </div>
                  </div>

                  <div class="glass-card rounded-[5px] py-[14px] px-[16px] flex flex-wrap gap-[10px]">
                    <button
                      class="h-[34px] px-[16px] rounded-[3px] bg-classic-blue text-white text-[13px] font-semibold cursor-pointer transition-opacity duration-150 hover:opacity-90 disabled:opacity-55"
                      disabled={launching()}
                      onClick={launch}
                    >
                      {launching() ? "启动中…" : isRunning(inst().id) ? "停止游戏" : "启动游戏"}
                    </button>
                    <button
                      class="h-[34px] px-[16px] rounded-[3px] border border-glass-border bg-glass-card text-classic-text text-[13px] cursor-pointer transition-colors duration-150 hover:bg-classic-blue-lightest hover:border-classic-blue-soft"
                      onClick={() => openInstanceDir(activeRoot(), inst().id)}
                    >
                      打开游戏目录
                    </button>
                    <button
                      class="h-[34px] px-[16px] rounded-[3px] border border-glass-border bg-glass-card text-classic-text text-[13px] cursor-pointer transition-colors duration-150 hover:bg-classic-blue-lightest hover:border-classic-blue-soft disabled:opacity-50 disabled:cursor-default"
                      disabled={busy() !== ""}
                      onClick={exportSelected}
                    >
                      {busy() === "export" ? "导出中…" : "导出整合包"}
                    </button>
                  </div>
                </div>
              </Show>

              {/* 实例管理:tab 由上方同级导航控制,面板内不再重复套一层。 */}
              <Show when={instanceTab() !== "home"}>
                <div class="glass-card rounded-[5px] flex-1 min-h-0 overflow-hidden">
                  <InstanceManageDialog
                    embedded
                    hideTabs
                    open
                    instance={inst()}
                    tab={activeManageTab()}
                    onTabChange={(tab) => setInstanceTab(tab)}
                    onClose={() => {}}
                    onChanged={async () => {
                      const list = await refetchInstances();
                      const cur = selected();
                      if (cur && list) setSelected(list.find((i) => i.id === cur.id) ?? cur);
                    }}
                    onCopied={async (newId) => {
                      const list = await refetchInstances();
                      setSelected((list ?? []).find((i) => i.id === newId) ?? null);
                      setInstanceTab("home");
                    }}
                  />
                </div>
              </Show>
            </div>
          )}
        </Show>

        {/* --- 启动日志(启动后) --- */}
        <Show when={rightView() === "log"}>
          <div>
            <h1 class="text-[20px] font-bold text-classic-text mt-0 mb-[3px] mx-0">{selected()?.name || selected()?.id || "游戏日志"}</h1>
            <Show when={selected()}>
              <p class="text-classic-text3 text-[13px] m-0">
                Minecraft {selected()!.mc_version} · {loaderLabel(selected()!.loader)}
                <Show when={selected()!.loader_version}>{` ${selected()!.loader_version}`}</Show>
              </p>
            </Show>
          </div>
          <div class="glass-card rounded-[5px] flex-1 min-h-0 flex flex-col overflow-hidden">
            <div class="py-[12px] px-[16px] text-[13px] font-bold text-classic-text border-b border-glass-divider">游戏日志</div>
            <pre class="flex-1 min-h-0 overflow-auto m-0 py-[12px] px-[16px] text-[12px]/[1.6] font-[ui-monospace,SFMono-Regular,Menlo,monospace] text-classic-text2 whitespace-pre-wrap [word-break:break-word]">
              <Show when={logs().length > 0} fallback={"启动后这里显示实时日志…"}>{logs().join("\n")}</Show>
            </pre>
          </div>
        </Show>
      </section>

      {/* 登录 / 切换账号弹窗 */}
      <Show when={showLogin()}>
        <ClassicAccountDialog
          onClose={() => setShowLogin(false)}
          onDone={() => {
            setShowLogin(false);
            refetchAccounts();
          }}
        />
      </Show>

      <Show when={importOutcome()}>
        {(o) => (
          <BlockedFilesDialog
            instanceId={o().instance_id}
            blocked={o().blocked}
            skipped={o().skipped_optional}
            onClose={() => setImportOutcome(null)}
          />
        )}
      </Show>

      {/* 从零新建实例:建好后直接打开它的详情视图 */}
      <NewInstanceDialog
        open={showNew()}
        onClose={() => setShowNew(false)}
        onCreated={async (id) => {
          const list = await refetchInstances();
          const created = (list ?? []).find((i) => i.id === id);
          if (created) openInstance(created);
        }}
      />
    </div>
  );
};

export default ClassicLaunch;
