import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { Avatar, Spinner, toast } from "../components";
import { api, onGameLog, onLaunchProgress } from "../ipc/api";
import { activeRoot } from "../store";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import type { AccountSummary, InstanceSummary } from "../ipc/types";
import PclAccountDialog from "./PclAccountDialog";
import "./PclLaunch.css";

const loaderLabel = (l: string) =>
  ({ vanilla: "原版", forge: "Forge", neoforge: "NeoForge", fabric: "Fabric", quilt: "Quilt" } as Record<string, string>)[l] ?? l;

const kindLabel = (k: string) =>
  k === "microsoft" ? "正版验证" : k === "yggdrasil" ? "外置登录" : "离线模式";

/** 右侧主区视图:新闻主页 / 版本选择 / 启动日志(照抄 PCL CE 的 启动页交互)。 */
type RightView = "news" | "versions" | "log";

/**
 * PclLaunch —— 1:1 照抄 PCL CE 启动页逻辑(非 Modrinth 那种列表页):
 *   - 左栏:账号卡(皮肤头像 + 用户名 + 验证方式)+ 招牌「启动游戏」按钮
 *     (副标题=当前版本)+「版本选择 / 版本设置」两个并排按钮。
 *   - 右栏:默认是「新闻主页」(欢迎卡 + 折叠卡 + 最新快照);点「版本选择」
 *     切到版本列表;启动后切到实时日志。版本列表不再常驻铺在启动页。
 */
const PclLaunch: Component = () => {
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
  const [newsOpen, setNewsOpen] = createSignal<Record<string, boolean>>({ snapshot: true });
  const [showLogin, setShowLogin] = createSignal(false);
  // 整合包导入/导出的进行态(禁用按钮 + 文案)。
  const [busy, setBusy] = createSignal<"" | "import" | "export">("");

  // 默认选中第一个版本(供启动按钮的副标题与启动用)。
  const pickDefault = (list: InstanceSummary[]) => {
    if (!selected() && list.length > 0) setSelected(list[0]);
    return list;
  };

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

  async function launch() {
    const inst = selected();
    if (!inst) {
      toast({ type: "error", message: "请先在「版本选择」里选一个版本" });
      setRightView("versions");
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
      toast({ type: "success", message: `已启动 ${inst.name}` });
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
      const blocked = out.blocked.length;
      toast({
        type: blocked > 0 ? "info" : "success",
        message:
          blocked > 0
            ? `已导入「${out.instance_id}」(${blocked} 个文件需手动下载)`
            : `已导入整合包「${out.instance_id}」`,
      });
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
    <div class="grid grid-cols-[300px_1fr] h-full min-h-0 bg-pcl-gray-bg">
      {/* ===== 左栏:账号卡 + 启动区(PCL 固定窄栏) ===== */}
      <aside class="grid grid-rows-[1fr_auto] min-h-0 bg-pcl-card border-r border-pcl-line">
        {/* 账号卡:皮肤头像 + 用户名 + 验证方式,点击打开登录/切换弹窗 */}
        <button
          class="flex flex-col items-center justify-center gap-[10px] p-[24px] text-center w-full border-none bg-transparent cursor-pointer transition-[background] duration-150 ease-[ease] hover:bg-pcl-blue-lightest"
          onClick={() => setShowLogin(true)}
          title="点击登录 / 切换账号"
        >
          <Show
            when={!accounts.loading}
            fallback={
              <div class="w-[84px] h-[84px] rounded-[10px] flex items-center justify-center text-[34px] font-extrabold text-white shadow-pcl [image-rendering:pixelated] bg-pcl-blue-bg2 animate-[pcl-pulse_1.3s_ease-in-out_infinite]" />
            }
          >
            <div class="w-[84px] h-[84px] rounded-[10px] flex items-center justify-center text-[34px] font-extrabold text-white shadow-pcl [image-rendering:pixelated] bg-[linear-gradient(135deg,var(--pcl-blue-hover),var(--pcl-blue))]">
              <Avatar kind={activeAccount()?.kind} uuid={activeAccount()?.uuid} />
            </div>
            <div class="text-[18px] font-bold text-pcl-text max-w-full whitespace-nowrap overflow-hidden text-ellipsis">
              {activeAccount()?.username ?? "未登录"}
            </div>
            <div class="text-[12px] text-pcl-text3">
              {activeAccount() ? kindLabel(activeAccount()!.kind) : "点击登录账号"}
            </div>
          </Show>
        </button>

        {/* 启动区:招牌按钮(描边 + 版本副标题)+ 版本选择/版本设置 */}
        <div class="pt-[14px] px-[20px] pb-[18px]">
          <button
            class="w-full h-[54px] flex flex-col items-center justify-center gap-px border-[1.5px] border-pcl-blue rounded-[3px] bg-pcl-blue-lightest cursor-pointer transition-[background,box-shadow,transform] duration-150 ease-[ease] enabled:hover:bg-pcl-blue-bg enabled:hover:shadow-[0_3px_10px_rgba(19,112,243,0.2)] enabled:active:scale-[0.99] disabled:opacity-[0.55] disabled:cursor-not-allowed"
            disabled={launching()}
            onClick={launch}
          >
            <span class="text-[16px] font-bold tracking-[1px] text-pcl-blue">{launching() ? "启动中…" : "启动游戏"}</span>
            <span class="text-[11px] text-pcl-text3 max-w-full whitespace-nowrap overflow-hidden text-ellipsis">
              <Show when={selected()} fallback={"未选择版本"}>
                {selected()!.name || selected()!.id}
              </Show>
            </span>
          </button>
          <div class="grid grid-cols-2 gap-[10px] mt-[10px]">
            <button
              class="h-[36px] border border-pcl-line rounded-[3px] bg-pcl-card text-pcl-text text-[13px] cursor-pointer transition-[background,border-color,color] duration-150 ease-[ease] hover:bg-pcl-blue-lightest hover:border-pcl-blue-soft"
              classList={{
                "bg-pcl-blue-bg": rightView() === "versions",
                "border-pcl-blue": rightView() === "versions",
                "text-pcl-blue": rightView() === "versions",
                "font-semibold": rightView() === "versions",
              }}
              onClick={() => setRightView(rightView() === "versions" ? "news" : "versions")}
            >
              版本选择
            </button>
            <button
              class="h-[36px] border border-pcl-line rounded-[3px] bg-pcl-card text-pcl-text text-[13px] cursor-pointer transition-[background,border-color,color] duration-150 ease-[ease] hover:bg-pcl-blue-lightest hover:border-pcl-blue-soft"
              onClick={() => toast({ type: "info", message: "版本设置:待接入" })}
            >
              版本设置
            </button>
          </div>
        </div>
      </aside>

      {/* ===== 右栏:新闻主页 / 版本选择 / 启动日志 ===== */}
      <section class="flex flex-col min-h-0 py-[18px] px-[22px] gap-[12px] overflow-auto">
        {/* --- 新闻主页(默认) --- */}
        <Show when={rightView() === "news"}>
          <div class="flex flex-col gap-[12px]">
            <div class="bg-pcl-card rounded-[5px] shadow-pcl py-[14px] px-[16px] flex items-center gap-[12px] bg-[linear-gradient(90deg,var(--pcl-blue-lightest),var(--pcl-card))] border-l-[3px] border-pcl-blue">
              <span class="text-[22px]">📰</span>
              <div>
                <div class="text-[14px] font-bold text-pcl-text">欢迎使用 PCL 启动器</div>
                <div class="text-[12px] text-pcl-text3 mt-[3px]">左侧选择账号与版本,点「启动游戏」即可开玩。</div>
              </div>
            </div>

            <button
              class="bg-pcl-card rounded-[5px] shadow-pcl py-[14px] px-[16px] flex items-center justify-between w-full border-none cursor-pointer text-left transition-[box-shadow] duration-150 ease-[ease] hover:shadow-pcl-strong"
              onClick={() => setNewsOpen((o) => ({ ...o, cape: !o.cape }))}
            >
              <span class="text-[14px] font-bold text-pcl-text">新披风与披风迁移</span>
              <span
                class="text-pcl-text3 text-[16px] transition-transform duration-200 ease-[ease]"
                classList={{ "rotate-180": !!newsOpen().cape }}
              >
                ⌄
              </span>
            </button>
            <Show when={newsOpen().cape}>
              <div class="bg-pcl-card rounded-[5px] shadow-pcl py-[14px] px-[16px] text-[13px] text-pcl-text2">
                <p class="m-0 leading-[1.7]">Mojang 已开放披风迁移。绑定正版账号后可在游戏内更换披风。</p>
              </div>
            </Show>

            <button
              class="bg-pcl-card rounded-[5px] shadow-pcl py-[14px] px-[16px] flex items-center justify-between w-full border-none cursor-pointer text-left transition-[box-shadow] duration-150 ease-[ease] hover:shadow-pcl-strong"
              onClick={() => setNewsOpen((o) => ({ ...o, snapshot: !o.snapshot }))}
            >
              <span class="text-[14px] font-bold text-pcl-text">
                最新快照版
                <Show when={latestSnapshot()}>{` - ${latestSnapshot()!.id}`}</Show>
              </span>
              <span
                class="text-pcl-text3 text-[16px] transition-transform duration-200 ease-[ease]"
                classList={{ "rotate-180": !!newsOpen().snapshot }}
              >
                ⌄
              </span>
            </button>
            <Show when={newsOpen().snapshot}>
              <div class="bg-pcl-card rounded-[5px] shadow-pcl py-[14px] px-[16px] text-[13px] text-pcl-text2">
                <div class="h-[150px] rounded-[5px] bg-[linear-gradient(135deg,#7fb0f7_0%,#4890f5_45%,#1370f3_100%)] flex items-center justify-center">
                  <Show when={latestSnapshot()} fallback={<Spinner />}>
                    <span class="py-[8px] px-[22px] rounded-[6px] bg-[rgba(11,91,203,0.85)] text-white text-[22px] font-bold tracking-[1px]">{latestSnapshot()!.id}</span>
                  </Show>
                </div>
                <div class="text-[14px] font-bold mt-[12px] text-pcl-blue">新特性</div>
                <Show when={latestSnapshot()}>
                  <ul class="list-disc mt-[6px] mb-0 pl-[18px] text-pcl-text2 text-[13px] leading-[1.9]">
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
          <div class="bg-pcl-card rounded-[5px] shadow-pcl flex flex-col min-h-0 overflow-hidden flex-1">
            <div class="flex items-center justify-between gap-[8px] py-[10px] px-[16px] text-[13px] font-bold text-pcl-text border-b border-pcl-line">
              <span>版本选择</span>
              <span class="flex gap-[6px]">
                <button
                  class="border border-pcl-blue text-pcl-blue bg-transparent rounded-[3px] py-[4px] px-[10px] text-[12px] font-semibold cursor-pointer transition-[background,color] duration-[var(--mo-dur-fast,150ms)] ease-[var(--mo-ease-out,ease)] enabled:hover:bg-pcl-blue enabled:hover:text-white disabled:opacity-50 disabled:cursor-default"
                  disabled={busy() !== ""}
                  onClick={importModpack}
                >
                  {busy() === "import" ? "导入中…" : "导入整合包"}
                </button>
                <button
                  class="border border-pcl-blue text-pcl-blue bg-transparent rounded-[3px] py-[4px] px-[10px] text-[12px] font-semibold cursor-pointer transition-[background,color] duration-[var(--mo-dur-fast,150ms)] ease-[var(--mo-ease-out,ease)] enabled:hover:bg-pcl-blue enabled:hover:text-white disabled:opacity-50 disabled:cursor-default"
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
                  fallback={<div class="py-[28px] px-[16px] text-pcl-text3 text-[13px] text-center leading-[1.9]">还没有版本<br />去「下载」装一个</div>}
                >
                  <For each={pickDefault(instances() ?? [])}>
                    {(inst) => (
                      <button
                        class="relative flex items-center gap-[10px] w-full h-[46px] pl-[16px] pr-[18px] border-none bg-transparent cursor-pointer text-left transition-[background] duration-150 ease-[ease] hover:bg-pcl-blue-bg2 before:content-[''] before:absolute before:left-0 before:top-[7px] before:bottom-[7px] before:w-[3px] before:rounded-[0_2px_2px_0] before:bg-transparent before:transition-[background] before:duration-150 before:ease-[ease] hover:before:bg-pcl-blue-soft"
                        classList={{
                          "bg-pcl-blue-bg": selected()?.id === inst.id,
                          "before:bg-pcl-blue": selected()?.id === inst.id,
                        }}
                        onClick={() => {
                          setSelected(inst);
                          setRightView("news");
                        }}
                      >
                        <span
                          class="w-[30px] h-[30px] flex-[0_0_30px] rounded-[4px] flex items-center justify-center font-bold text-[14px] text-white bg-pcl-blue data-[loader=forge]:bg-[#c96a1c] data-[loader=neoforge]:bg-[#c96a1c] data-[loader=fabric]:bg-[#a87b3f] data-[loader=quilt]:bg-[#a87b3f]"
                          data-loader={inst.loader}
                        >
                          {(inst.name || inst.id)[0]?.toUpperCase()}
                        </span>
                        <span class="flex flex-col min-w-0">
                          <span class="text-pcl-text text-[13px] whitespace-nowrap overflow-hidden text-ellipsis">{inst.name || inst.id}</span>
                          <span class="text-pcl-text3 text-[11px]">{inst.mc_version} · {loaderLabel(inst.loader)}</span>
                        </span>
                      </button>
                    )}
                  </For>
                </Show>
              </Show>
            </div>
          </div>
        </Show>

        {/* --- 启动日志(启动后) --- */}
        <Show when={rightView() === "log"}>
          <div>
            <h1 class="text-[20px] font-bold text-pcl-text mt-0 mb-[3px] mx-0">{selected()?.name || selected()?.id || "游戏日志"}</h1>
            <Show when={selected()}>
              <p class="text-pcl-text3 text-[13px] m-0">
                Minecraft {selected()!.mc_version} · {loaderLabel(selected()!.loader)}
                <Show when={selected()!.loader_version}>{` ${selected()!.loader_version}`}</Show>
              </p>
            </Show>
          </div>
          <div class="bg-pcl-card rounded-[5px] shadow-pcl flex-1 min-h-0 flex flex-col overflow-hidden">
            <div class="py-[12px] px-[16px] text-[13px] font-bold text-pcl-text border-b border-pcl-line">游戏日志</div>
            <pre class="flex-1 min-h-0 overflow-auto m-0 py-[12px] px-[16px] text-[12px]/[1.6] font-[ui-monospace,SFMono-Regular,Menlo,monospace] text-pcl-text2 whitespace-pre-wrap [word-break:break-word]">
              <Show when={logs().length > 0} fallback={"启动后这里显示实时日志…"}>{logs().join("\n")}</Show>
            </pre>
          </div>
        </Show>
      </section>

      {/* 登录 / 切换账号弹窗 */}
      <Show when={showLogin()}>
        <PclAccountDialog
          onClose={() => setShowLogin(false)}
          onDone={() => {
            setShowLogin(false);
            refetchAccounts();
          }}
        />
      </Show>
    </div>
  );
};

export default PclLaunch;
