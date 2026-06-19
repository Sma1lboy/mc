import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { Spinner, toast } from "../components";
import { api, onGameLog, onLaunchProgress } from "../ipc/api";
import { currentRoot } from "../store";
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
    () => currentRoot() ?? "",
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
  const avatarInitial = () => (activeAccount()?.username?.[0] ?? "?").toUpperCase();

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
      await api.launchInstance(currentRoot() ?? "", inst.id, name, online);
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
      const out = await api.importModpack(currentRoot() ?? "", picked, null);
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
        root: currentRoot() ?? "",
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
    <div class="pcl-launch">
      {/* ===== 左栏:账号卡 + 启动区(PCL 固定窄栏) ===== */}
      <aside class="pcl-side">
        {/* 账号卡:皮肤头像 + 用户名 + 验证方式,点击打开登录/切换弹窗 */}
        <button class="pcl-account" onClick={() => setShowLogin(true)} title="点击登录 / 切换账号">
          <Show
            when={!accounts.loading}
            fallback={<div class="pcl-avatar pcl-avatar-skel" />}
          >
            <div class="pcl-avatar">{avatarInitial()}</div>
            <div class="pcl-account-name">{activeAccount()?.username ?? "未登录"}</div>
            <div class="pcl-account-kind">
              {activeAccount() ? kindLabel(activeAccount()!.kind) : "点击登录账号"}
            </div>
          </Show>
        </button>

        {/* 启动区:招牌按钮(描边 + 版本副标题)+ 版本选择/版本设置 */}
        <div class="pcl-launchbar">
          <button class="pcl-launchbtn" disabled={launching()} onClick={launch}>
            <span class="pcl-launchbtn-main">{launching() ? "启动中…" : "启动游戏"}</span>
            <span class="pcl-launchbtn-sub">
              <Show when={selected()} fallback={"未选择版本"}>
                {selected()!.name || selected()!.id}
              </Show>
            </span>
          </button>
          <div class="pcl-subbtns">
            <button
              class="pcl-subbtn"
              classList={{ active: rightView() === "versions" }}
              onClick={() => setRightView(rightView() === "versions" ? "news" : "versions")}
            >
              版本选择
            </button>
            <button class="pcl-subbtn" onClick={() => toast({ type: "info", message: "版本设置:待接入" })}>
              版本设置
            </button>
          </div>
        </div>
      </aside>

      {/* ===== 右栏:新闻主页 / 版本选择 / 启动日志 ===== */}
      <section class="pcl-main">
        {/* --- 新闻主页(默认) --- */}
        <Show when={rightView() === "news"}>
          <div class="pcl-news">
            <div class="pcl-news-card pcl-news-welcome">
              <span class="pcl-news-ico">📰</span>
              <div>
                <div class="pcl-news-h">欢迎使用 PCL 启动器</div>
                <div class="pcl-news-p">左侧选择账号与版本,点「启动游戏」即可开玩。</div>
              </div>
            </div>

            <button
              class="pcl-news-card pcl-news-collapse"
              onClick={() => setNewsOpen((o) => ({ ...o, cape: !o.cape }))}
            >
              <span class="pcl-news-h">新披风与披风迁移</span>
              <span class="pcl-chev" classList={{ open: !!newsOpen().cape }}>⌄</span>
            </button>
            <Show when={newsOpen().cape}>
              <div class="pcl-news-card pcl-news-body">
                <p>Mojang 已开放披风迁移。绑定正版账号后可在游戏内更换披风。</p>
              </div>
            </Show>

            <button
              class="pcl-news-card pcl-news-collapse"
              onClick={() => setNewsOpen((o) => ({ ...o, snapshot: !o.snapshot }))}
            >
              <span class="pcl-news-h">
                最新快照版
                <Show when={latestSnapshot()}>{` - ${latestSnapshot()!.id}`}</Show>
              </span>
              <span class="pcl-chev" classList={{ open: !!newsOpen().snapshot }}>⌄</span>
            </button>
            <Show when={newsOpen().snapshot}>
              <div class="pcl-news-card pcl-news-body">
                <div class="pcl-news-banner">
                  <Show when={latestSnapshot()} fallback={<Spinner />}>
                    <span class="pcl-news-chip">{latestSnapshot()!.id}</span>
                  </Show>
                </div>
                <div class="pcl-news-h" style={{ "margin-top": "12px", color: "var(--pcl-blue)" }}>新特性</div>
                <Show when={latestSnapshot()}>
                  <ul class="pcl-news-list">
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
          <div class="pcl-vpane">
            <div class="pcl-vpane-head">
              <span>版本选择</span>
              <span class="pcl-vpane-actions">
                <button class="pcl-vaction" disabled={busy() !== ""} onClick={importModpack}>
                  {busy() === "import" ? "导入中…" : "导入整合包"}
                </button>
                <button
                  class="pcl-vaction"
                  disabled={busy() !== "" || !selected()}
                  onClick={exportSelected}
                >
                  {busy() === "export" ? "导出中…" : "导出整合包"}
                </button>
              </span>
            </div>
            <div class="pcl-vlist">
              <Show when={!instances.loading} fallback={<div class="pcl-vloading"><Spinner /></div>}>
                <Show
                  when={(instances() ?? []).length > 0}
                  fallback={<div class="pcl-vempty">还没有版本<br />去「下载」装一个</div>}
                >
                  <For each={pickDefault(instances() ?? [])}>
                    {(inst) => (
                      <button
                        class="pcl-vrow"
                        classList={{ active: selected()?.id === inst.id }}
                        onClick={() => {
                          setSelected(inst);
                          setRightView("news");
                        }}
                      >
                        <span class="pcl-vicon" data-loader={inst.loader}>
                          {(inst.name || inst.id)[0]?.toUpperCase()}
                        </span>
                        <span class="pcl-vtext">
                          <span class="pcl-vname">{inst.name || inst.id}</span>
                          <span class="pcl-vmeta">{inst.mc_version} · {loaderLabel(inst.loader)}</span>
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
          <div class="pcl-detail">
            <h1>{selected()?.name || selected()?.id || "游戏日志"}</h1>
            <Show when={selected()}>
              <p>
                Minecraft {selected()!.mc_version} · {loaderLabel(selected()!.loader)}
                <Show when={selected()!.loader_version}>{` ${selected()!.loader_version}`}</Show>
              </p>
            </Show>
          </div>
          <div class="pcl-card pcl-logcard">
            <div class="pcl-card-title">游戏日志</div>
            <pre class="pcl-log-body">
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
