import { useEffect, useMemo, useState } from "react";
import {
  BlockedFilesDialog,
  EmptyState,
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
import { useAppStore, openInstance, openDiscover, playInstance, refreshInstances } from "../store";
import { useAsync } from "../util/useAsync";
import { useModpackDrop } from "../util/useModpackDrop";
import { sortByRecent } from "../util/instances";
import { loaderLabel } from "../util/loaders";
import { t } from "../i18n";
import { useLang } from "../i18n";
import type { ImportOutcome, InstanceSummary, SearchHit } from "../ipc/types";

/**
 * Home —— Blocky Craft 工作台首页(规范 §4.1)。
 *   - 顶栏:欢迎语 + 上次游玩副行;右上账号芯片。
 *   - 继续游玩:最近一个实例的主入口大卡(CONTINUE)。
 *   - 最近实例:其余实例的紧凑卡网格。
 *   - 发现整合包:Modrinth 热门整合包封面卡。
 * 数据全部经 store / useAsync,随 currentRoot 变化自动重载。
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

export default function Home() {
  useLang();

  // 实例列表来自全局 store(单一真相,见 store.ts);依赖 currentRoot,切根自动重拉。
  // undefined = 尚未加载(对齐旧 resource 的「未就绪」= loading 语义)。
  const instances = useAppStore((s) => s.instances);
  const runningIds = useAppStore((s) => s.runningIds);
  const currentRoot = useAppStore((s) => s.currentRoot);

  // 整合包推荐:一次性拉取热门 modpack。
  const packs = useAsync(
    () =>
      api
        .modrinthSearch("", "modpack", null, null, null, null, null, null, null)
        .catch(() => [] as SearchHit[]),
    [],
  );

  // 导入整合包:把文件拖到本页任意处即导入(打开弹窗并自动开始);弹窗已开时让它自己接管。
  const [importOpen, setImportOpen] = useState(false);
  const [importPath, setImportPath] = useState<string | null>(null);
  const [importOutcome, setImportOutcome] = useState<ImportOutcome | null>(null);
  function handleImported(out: ImportOutcome) {
    refreshInstances();
    if (out.blocked.length > 0 || out.skipped_optional.length > 0) setImportOutcome(out);
    else toast({ type: "success", message: t("library.imported", { id: out.instance_id }) });
  }
  const dragOver = useModpackDrop({
    enabled: !importOpen,
    onFile: (path) => {
      setImportPath(path);
      setImportOpen(true);
    },
    onUnsupported: () => toast({ type: "info", message: t("components.import.unsupported") }),
  });

  // 启动反馈:仅订阅进度提示。成功/退出/崩溃的 toast 与运行态由 store 统一处理
  //(基于真实的 game://started/exit 事件,而非「第一行日志」这种会把崩溃误报成成功的信号)。
  // 后端可能把同一启动 stage 连发多次;去重,避免「启动游戏进程」弹两遍。
  useEffect(() => {
    let lastStage = "";
    return onLaunchProgress((p) => {
      if (p.stage && p.stage !== lastStage) {
        lastStage = p.stage;
        toast({ type: "info", message: p.stage });
      }
    });
  }, []);

  // Home 只当快捷入口:按最近游玩排序。第一个进「继续游玩」大卡,其余进「最近实例」网格(取前 6)。
  const RECENT_CAP = 6;
  const sortedByPlayed = useMemo(() => sortByRecent(instances ?? []), [instances]);
  const featured = sortedByPlayed[0];
  const recent = sortedByPlayed.slice(1, 1 + RECENT_CAP);

  // 副行:上次游玩的整合包名 · 相对时间;无任何实例时给引导文案。
  const welcomeSub = (): string => {
    if (!featured) return t("home.welcomeSubEmpty");
    const name = featured.name || featured.id;
    const rel = formatRelativeTime(featured.last_played ?? 0);
    return rel === "never"
      ? t("home.welcomeSubNever", { name })
      : t("home.welcomeSub", { name, rel });
  };

  return (
    <div className="relative py-[24px] px-[28px] overflow-y-auto h-full">
      {dragOver && (
        <div className="absolute inset-0 z-30 flex items-center justify-center bg-black/40 backdrop-blur-sm pointer-events-none">
          <Panel
            variant="raised"
            className="flex flex-col items-center gap-[10px] border-2 border-dashed border-accent px-[40px] py-[32px]"
          >
            <Icon name="download" size={30} className="text-accent" />
            <div className="text-[14px] font-medium text-fg">{t("components.import.dropOverlay")}</div>
          </Panel>
        </div>
      )}

      {/* ===== ① 顶栏:欢迎语 + 副行 / 账号芯片 ===== */}
      <header className="flex items-start justify-between gap-[16px] mb-[24px]">
        <div className="min-w-0">
          <Heading size="page" as="h1">{t("library.welcomeBack")}</Heading>
          <div className="mt-[6px] text-[13px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
            {welcomeSub()}
          </div>
        </div>

        {/* 右上账号芯片:点开切换/添加账号(AccountMenu 自带列表 + 登录弹窗)。 */}
        <AccountMenu variant="chip" />
      </header>

      {/* ===== ② 继续游玩 大卡 ===== */}
      {instances === undefined ? (
        <div className="flex justify-center p-[40px]"><Spinner /></div>
      ) : (
        <>
          {featured ? (
            <section className="mb-[28px]">
              <Heading size="section" as="h2" className="mb-[12px]">{t("library.continuePlaying")}</Heading>
              <Panel variant="sunken" className="flex items-stretch gap-[18px] p-[18px]">
                {/* 左:封面占位(斜纹方块) / 自定义图标 */}
                <Panel
                  variant="input"
                  className="relative w-[160px] shrink-0 self-stretch min-h-[140px] overflow-hidden cursor-pointer"
                  onClick={() => openInstance(featured.id)}
                >
                  {featured.icon ? (
                    <div className="absolute inset-0">
                      <InstanceIcon name={featured.name || featured.id} icon={featured.icon ?? undefined} />
                    </div>
                  ) : (
                    <div
                      className="absolute inset-0"
                      style={{
                        background:
                          "repeating-linear-gradient(45deg, var(--panel-2) 0, var(--panel-2) 10px, var(--panel-3) 10px, var(--panel-3) 20px)",
                      }}
                      aria-hidden="true"
                    />
                  )}
                </Panel>

                {/* 右:CONTINUE 徽标 + 名称 + 元信息 + 操作 */}
                <div className="flex flex-col min-w-0 flex-1 gap-[10px]">
                  <PixelLabel className="self-start bg-accent text-accent-text px-[8px] py-[5px]">
                    {t("home.continueBadge")}
                  </PixelLabel>
                  <Heading
                    size="section"
                    className="text-strong truncate cursor-pointer"
                    title={featured.name || featured.id}
                  >
                    <span onClick={() => openInstance(featured.id)}>{featured.name || featured.id}</span>
                  </Heading>
                  <div className="text-[13px] text-sub">{metaLine(featured)}</div>
                  <div className="flex items-center gap-[10px] mt-auto pt-[6px]">
                    <PlayButton
                      running={runningIds.has(featured.id)}
                      onClick={() => void playInstance(featured.id)}
                    />
                    <Button variant="ghost" onClick={() => openInstance(featured.id)}>
                      {t("home.details")}
                    </Button>
                  </div>
                </div>
              </Panel>
            </section>
          ) : (
            <EmptyState
              title={<>{t("library.emptyHomePrefix")}<b>{t("library.emptyHomeLink")}</b>{t("library.emptyHomeSuffix")}</>}
            />
          )}

          {/* ===== ③ 最近实例 网格 ===== */}
          {recent.length > 0 && (
            <section className="mb-[28px]">
              <Heading size="section" as="h2" className="mb-[12px]">{t("layout.recentInstances")}</Heading>
              <div className="grid grid-cols-3 gap-[12px]">
                {recent.map((inst) => (
                  <Panel
                    key={inst.id}
                    variant="raised"
                    className="group flex items-center gap-[11px] p-[11px] cursor-pointer"
                    onClick={() => openInstance(inst.id)}
                  >
                    <Panel variant="input" className="w-[44px] h-[44px] shrink-0 overflow-hidden">
                      <InstanceIcon name={inst.name || inst.id} icon={inst.icon ?? undefined} />
                    </Panel>
                    <div className="flex flex-col min-w-0 flex-1">
                      <span
                        className="font-display text-[14px] text-strong leading-tight truncate"
                        title={inst.name || inst.id}
                      >
                        {inst.name || inst.id}
                      </span>
                      <span className="text-[12px] text-muted truncate">{metaLine(inst)}</span>
                    </div>
                    {/* 小橙播放键:运行中转红方块,点击启动/停止(阻止冒泡,避免同时进详情)。 */}
                    <button
                      type="button"
                      className={`shrink-0 w-[30px] h-[30px] grid place-items-center text-[#ffffff] shadow-raised active:shadow-pressed border-none cursor-pointer ${
                        runningIds.has(inst.id) ? "bg-danger hover:bg-danger-hover" : "bg-accent hover:bg-accent-hover"
                      }`}
                      title={runningIds.has(inst.id) ? t("components.play.stop") : t("components.play.start")}
                      onClick={(e) => {
                        e.stopPropagation();
                        void playInstance(inst.id);
                      }}
                    >
                      {runningIds.has(inst.id) ? (
                        <svg width="10" height="10" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
                          <rect x="1.5" y="1.5" width="9" height="9" rx="0.5" />
                        </svg>
                      ) : (
                        <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
                          <path d="M2.5 1.6c0-.5.55-.82.99-.57l6.9 3.97c.45.26.45.92 0 1.18l-6.9 3.97a.66.66 0 0 1-.99-.57V1.6Z" />
                        </svg>
                      )}
                    </button>
                  </Panel>
                ))}
              </div>
            </section>
          )}
        </>
      )}

      {/* ===== ④ 发现整合包 ===== */}
      <section className="mb-[8px]">
        <div className="flex items-center justify-between mb-[12px]">
          <Heading size="section" as="h2">{t("home.discoverHeading")}</Heading>
          <button
            className="text-[13px] text-accent bg-transparent border-none cursor-pointer hover:text-accent-hover transition-colors duration-150"
            onClick={() => openDiscover()}
          >
            {t("library.viewAll")}
          </button>
        </div>
        {packs.loading ? (
          <div className="flex justify-center p-[32px]"><Spinner /></div>
        ) : (
          <div className="grid grid-cols-2 gap-[16px]">
            {(packs.data ?? []).slice(0, 6).map((hit) => (
              <ModpackCard
                key={hit.id}
                hit={toHit(hit)}
                onClick={(h) => openDiscover({ hit: h, kind: "modpack" })}
              />
            ))}
          </div>
        )}
      </section>

      <ImportModpackDialog
        open={importOpen}
        root={currentRoot ?? ""}
        initialPath={importPath}
        onClose={() => {
          setImportOpen(false);
          setImportPath(null);
        }}
        onImported={handleImported}
      />

      {importOutcome && (
        <BlockedFilesDialog
          instanceId={importOutcome.instance_id}
          blocked={importOutcome.blocked}
          skipped={importOutcome.skipped_optional}
          onClose={() => setImportOutcome(null)}
        />
      )}
    </div>
  );
}
