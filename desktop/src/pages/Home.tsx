import { Component, createMemo, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import {
  BlockedFilesDialog,
  EmptyState,
  ErrorState,
  Heading,
  Icon,
  ImportModpackDialog,
  InstanceIcon,
  ModpackCard,
  Panel,
  PixelLabel,
  PlayButton,
  AccountMenu,
  Button,
  Spinner,
  toast,
  formatRelativeTime,
  type ModpackHit,
} from "../components";
import { api, onLaunchProgress } from "../ipc/api";
import {
  activeRoot,
  openInstance,
  openDiscover,
  playInstance,
  instances,
  refreshInstances,
  isRunning,
} from "../store";
import { useModpackDrop } from "../util/useModpackDrop";
import { sortByRecent } from "../util/instances";
import { loaderLabel } from "../util/loaders";
import { t } from "../i18n";
import type { ImportOutcome, InstanceSummary, SearchHit } from "../ipc/types";

/**
 * Home —— Blocky Craft 工作台首页(规范 §4.1)。
 *   - 顶栏:欢迎语 + 上次游玩副行;右上账号芯片。
 *   - 继续游玩:最近一个实例的主入口大卡(CONTINUE)。
 *   - 最近实例:其余实例的紧凑卡网格。
 *   - 发现整合包:Modrinth 热门整合包封面卡。
 * 数据全部经 store / createResource,随 currentRoot 变化自动重载。
 */

function toHit(h: SearchHit): ModpackHit {
  return {
    id: h.id,
    slug: h.slug,
    title: h.title,
    description: h.description,
    author: h.author,
    downloads: h.downloads,
    icon_url: h.icon_url || undefined,
    gallery_url: h.gallery_url || undefined,
    categories: h.categories,
  };
}

// 实例 → 「加载器 + 版本」短串(如 "Fabric 1.20.1");无加载器名时只显版本。
function metaLine(i: InstanceSummary): string {
  const name = loaderLabel(i.loader);
  return name ? `${name} ${i.mc_version}` : i.mc_version;
}

const Home: Component = () => {
  // 实例列表来自全局 store(单一真相,见 store.ts);依赖 currentRoot,切根自动重拉。

  // 整合包推荐:一次性拉取热门 modpack。
  const [packs] = createResource(() =>
    api.modrinthSearch("", "modpack", null, null, null, null, null, null, null).catch(() => [] as SearchHit[]),
  );

  // 导入整合包:把文件拖到本页任意处即导入(打开弹窗并自动开始);弹窗已开时让它自己接管。
  const [importOpen, setImportOpen] = createSignal(false);
  const [importPath, setImportPath] = createSignal<string | null>(null);
  const [importOutcome, setImportOutcome] = createSignal<ImportOutcome | null>(null);
  function handleImported(out: ImportOutcome) {
    refreshInstances();
    if (out.blocked.length > 0 || out.skipped_optional.length > 0) setImportOutcome(out);
    else toast({ type: "success", message: t("library.imported", { id: out.instance_id }) });
  }
  const dragOver = useModpackDrop({
    enabled: () => !importOpen(),
    onFile: (path) => {
      setImportPath(path);
      setImportOpen(true);
    },
    onUnsupported: () => toast({ type: "info", message: t("components.import.unsupported") }),
  });

  // 启动反馈:仅订阅进度提示。成功/退出/崩溃的 toast 与运行态由 store 统一处理
  //(基于真实的 game://started/exit 事件,而非「第一行日志」这种会把崩溃误报成成功的信号)。
  // 后端可能把同一启动 stage 连发多次;去重,避免「启动游戏进程」弹两遍。
  let lastStage = "";
  const offProgress = onLaunchProgress((p) => {
    if (p.stage && p.stage !== lastStage) {
      lastStage = p.stage;
      toast({ type: "info", message: p.stage });
    }
  });
  onCleanup(() => {
    offProgress();
  });

  // Home 只当快捷入口:按最近游玩排序。第一个进「继续游玩」大卡,其余进「最近实例」网格(取前 6)。
  const RECENT_CAP = 6;
  const sortedByPlayed = createMemo(() => sortByRecent(instances() ?? []));
  const featured = () => sortedByPlayed()[0];
  const recent = () => sortedByPlayed().slice(1, 1 + RECENT_CAP);

  // 副行:上次游玩的整合包名 · 相对时间;无任何实例时给引导文案。
  const welcomeSub = () => {
    const f = featured();
    if (!f) return t("home.welcomeSubEmpty");
    const name = f.name || f.id;
    const rel = formatRelativeTime(f.last_played ?? 0);
    return rel === "never"
      ? t("home.welcomeSubNever", { name })
      : t("home.welcomeSub", { name, rel });
  };

  return (
    <div class="relative py-[24px] px-[28px] overflow-y-auto h-full">
      <Show when={dragOver()}>
        <div class="absolute inset-0 z-30 flex items-center justify-center bg-black/40 backdrop-blur-sm pointer-events-none">
          <Panel
            variant="raised"
            class="flex flex-col items-center gap-[10px] border-2 border-dashed border-accent px-[40px] py-[32px]"
          >
            <Icon name="download" size={30} class="text-accent" />
            <div class="text-[14px] font-medium text-fg">{t("components.import.dropOverlay")}</div>
          </Panel>
        </div>
      </Show>

      {/* ===== ① 顶栏:欢迎语 + 副行 / 账号芯片 ===== */}
      <header class="flex items-start justify-between gap-[16px] mb-[24px]">
        <div class="min-w-0">
          <Heading size="page" as="h1">{t("library.welcomeBack")}</Heading>
          <div class="mt-[6px] text-[13px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
            {welcomeSub()}
          </div>
        </div>

        {/* 右上账号芯片:点开切换/添加账号(AccountMenu 自带列表 + 登录弹窗)。 */}
        <AccountMenu variant="chip" />
      </header>

      {/* ===== ② 继续游玩 大卡 ===== */}
      <Show
        when={!instances.loading}
        fallback={<div class="flex justify-center p-[40px]"><Spinner /></div>}
      >
        <Show
          when={!instances.error}
          fallback={
            <ErrorState message={t("library.instanceListError")} onRetry={() => void refreshInstances()} />
          }
        >
          <Show
            when={featured()}
            fallback={
              <EmptyState
                title={<>{t("library.emptyHomePrefix")}<b>{t("library.emptyHomeLink")}</b>{t("library.emptyHomeSuffix")}</>}
              />
            }
          >
            {(f) => (
              <section class="mb-[28px]">
                <Heading size="section" as="h2" class="mb-[12px]">{t("library.continuePlaying")}</Heading>
                <Panel variant="sunken" class="flex items-stretch gap-[18px] p-[18px]">
                  {/* 左:封面占位(斜纹方块) / 自定义图标 */}
                  <Panel
                    variant="input"
                    class="relative w-[160px] shrink-0 self-stretch min-h-[140px] overflow-hidden cursor-pointer"
                    onClick={() => openInstance(f().id)}
                  >
                    <Show
                      when={f().icon}
                      fallback={
                        <div
                          class="absolute inset-0"
                          style={{
                            background:
                              "repeating-linear-gradient(45deg, var(--panel-2) 0, var(--panel-2) 10px, var(--panel-3) 10px, var(--panel-3) 20px)",
                          }}
                          aria-hidden="true"
                        />
                      }
                    >
                      <div class="absolute inset-0">
                        <InstanceIcon name={f().name || f().id} icon={f().icon ?? undefined} />
                      </div>
                    </Show>
                  </Panel>

                  {/* 右:CONTINUE 徽标 + 名称 + 元信息 + 操作 */}
                  <div class="flex flex-col min-w-0 flex-1 gap-[10px]">
                    <PixelLabel class="self-start bg-accent text-accent-text px-[8px] py-[5px]">
                      {t("home.continueBadge")}
                    </PixelLabel>
                    <Heading
                      size="section"
                      class="text-strong truncate cursor-pointer"
                      title={f().name || f().id}
                    >
                      <span onClick={() => openInstance(f().id)}>{f().name || f().id}</span>
                    </Heading>
                    <div class="text-[13px] text-sub">{metaLine(f())}</div>
                    <div class="flex items-center gap-[10px] mt-auto pt-[6px]">
                      <PlayButton
                        running={isRunning(f().id)}
                        onClick={() => void playInstance(f().id)}
                      />
                      <Button variant="ghost" onClick={() => openInstance(f().id)}>
                        {t("home.details")}
                      </Button>
                    </div>
                  </div>
                </Panel>
              </section>
            )}
          </Show>

          {/* ===== ③ 最近实例 网格 ===== */}
          <Show when={recent().length > 0}>
            <section class="mb-[28px]">
              <Heading size="section" as="h2" class="mb-[12px]">{t("layout.recentInstances")}</Heading>
              <div class="grid grid-cols-3 gap-[12px]">
                <For each={recent()}>
                  {(inst) => (
                    <Panel
                      variant="raised"
                      class="group flex items-center gap-[11px] p-[11px] cursor-pointer"
                      onClick={() => openInstance(inst.id)}
                    >
                      <Panel
                        variant="input"
                        class="w-[44px] h-[44px] shrink-0 overflow-hidden"
                      >
                        <InstanceIcon name={inst.name || inst.id} icon={inst.icon ?? undefined} />
                      </Panel>
                      <div class="flex flex-col min-w-0 flex-1">
                        <span
                          class="font-display text-[14px] text-strong leading-tight truncate"
                          title={inst.name || inst.id}
                        >
                          {inst.name || inst.id}
                        </span>
                        <span class="text-[12px] text-muted truncate">{metaLine(inst)}</span>
                      </div>
                      {/* 小橙播放键:运行中转红方块,点击启动/停止(阻止冒泡,避免同时进详情)。 */}
                      <button
                        type="button"
                        class={`shrink-0 w-[30px] h-[30px] grid place-items-center text-[#ffffff] shadow-raised active:shadow-pressed border-none cursor-pointer ${
                          isRunning(inst.id) ? "bg-danger hover:bg-danger-hover" : "bg-accent hover:bg-accent-hover"
                        }`}
                        title={isRunning(inst.id) ? t("components.play.stop") : t("components.play.start")}
                        onClick={(e) => {
                          e.stopPropagation();
                          void playInstance(inst.id);
                        }}
                      >
                        <Show
                          when={isRunning(inst.id)}
                          fallback={
                            <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
                              <path d="M2.5 1.6c0-.5.55-.82.99-.57l6.9 3.97c.45.26.45.92 0 1.18l-6.9 3.97a.66.66 0 0 1-.99-.57V1.6Z" />
                            </svg>
                          }
                        >
                          <svg width="10" height="10" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
                            <rect x="1.5" y="1.5" width="9" height="9" rx="0.5" />
                          </svg>
                        </Show>
                      </button>
                    </Panel>
                  )}
                </For>
              </div>
            </section>
          </Show>
        </Show>
      </Show>

      {/* ===== ④ 发现整合包 ===== */}
      <section class="mb-[8px]">
        <div class="flex items-center justify-between mb-[12px]">
          <Heading size="section" as="h2">{t("home.discoverHeading")}</Heading>
          <button
            class="text-[13px] text-accent bg-transparent border-none cursor-pointer hover:text-accent-hover transition-colors duration-150"
            onClick={() => openDiscover()}
          >
            {t("library.viewAll")}
          </button>
        </div>
        <Show
          when={!packs.loading}
          fallback={<div class="flex justify-center p-[32px]"><Spinner /></div>}
        >
          <div class="grid grid-cols-2 gap-[16px]">
            <For each={(packs() ?? []).slice(0, 6)}>
              {(hit) => (
                <ModpackCard
                  hit={toHit(hit)}
                  onClick={(h) => openDiscover({ hit: h, kind: "modpack" })}
                />
              )}
            </For>
          </div>
        </Show>
      </section>

      <ImportModpackDialog
        open={importOpen()}
        root={activeRoot()}
        initialPath={importPath()}
        onClose={() => {
          setImportOpen(false);
          setImportPath(null);
        }}
        onImported={handleImported}
      />

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
    </div>
  );
};

export default Home;
