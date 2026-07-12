import { useEffect, useRef, useState } from "react";
import { BookOpen } from "lucide-react";
import { InstanceManageDialog, InstanceIcon, Dialog, ExportModpackDialog, toast, Button, Chip, Heading, PixelLabel, Tag, type InstanceRowData } from "../components";
import { PlayButton } from "../components/PlayButton";
import { RealmPanel } from "../components/RealmPanel";
import { Menu } from "../components/Menu";
import { formatRelativeTime } from "../components/format";
import { api, onInstallProgress } from "../ipc/api";
import { cached } from "../ipc/cache";
import { useAsync } from "../util/useAsync";
import { openInstanceDir, deleteInstance } from "../util/instanceActions";
import { loaderLabel as fmtLoader } from "../util/loaders";
import { useAppStore, activeRoot, isRunning, playInstance, closeInstance, openInstance, refreshInstances } from "../store";
import { renderMarkdown } from "../util/markdown";
import { t, useLang } from "../i18n";
import { openAgentChat } from "../agent/chatStore";
import "./ModpackDetail.css"; // .md 样式(整合包更新日志渲染)

/**
 * InstanceDetail —— 实例详情页(替代旧的管理弹窗):
 *   顶部头部(返回 / 图标 / 名称 / 版本·加载器 / 大 Play)
 *   下方复用 InstanceManageDialog(embedded:tabs 设置/Mods/资源包/光影/数据包/存档/截图)。
 * 由 store.openInstance(id) 进入,closeInstance() 返回来源页。
 */
export default function InstanceDetail() {
  useLang();
  // 从全局 store 的 instances 派生,别再自持一份"整机实例列表"的 resource:开详情时
  // 就不会多打一次 listInstances,也不会在 pending 时闪 t('instance.loading')——数据本就
  // 在内存里(store 声明它是 library/home/rail/安装目标 的唯一来源)。刷新走 refreshInstances()。
  const instanceList = useAppStore((s) => s.instances);
  const currentId = useAppStore((s) => s.currentInstanceId);
  const running = useAppStore((s) => s.runningIds);
  const launching = useAppStore((s) => s.launchingIds);
  const socialOn = useAppStore((s) => s.socialEnabled);
  const inst = () => (instanceList ?? []).find((i) => i.id === currentId) ?? null;

  // 整合包更新检查(仅对由 Modrinth 整合包安装的实例返回非空);失败/无来源都安静返回空。
  const { data: updatesData, refetch: refetchUpdates } = useAsync(
    () => (currentId ? api.checkModpackUpdates(activeRoot(), currentId).catch(() => []) : Promise.resolve([])),
    [currentId],
  );
  const updates = () => updatesData;
  const { data: cfgData } = useAsync(
    () => (currentId ? api.getInstanceConfig(activeRoot(), currentId).catch(() => null) : Promise.resolve(null)),
    [currentId],
  );
  const cfg = () => cfgData;
  // 整合包来源(Modrinth / CurseForge)→ 拉项目详情,用真实 logo + 下载量/收藏数 + 分类
  // 点亮实例头部(避免和「概览」标签重复展示同一套品牌信息)。
  const cfgSource = cfgData?.source;
  const srcProvider =
    cfgSource && (cfgSource.provider === "modrinth" || cfgSource.provider === "curseforge")
      ? cfgSource.provider
      : null;
  const projectId = srcProvider ? cfgSource!.project_id : null;
  const { data: projectData } = useAsync(
    () =>
      projectId && srcProvider
        ? cached(`project|${srcProvider}|${projectId}`, () => api.modrinthProject(projectId, srcProvider)).catch(() => null)
        : Promise.resolve(null),
    [projectId, srcProvider],
  );
  const project = () => projectData;
  // 早于「安装即存图标」的整合包实例本地没 icon.png:发现缺失且项目有 logo 时补齐一次,
  // 刷新后侧栏/首页/详情都用上真实 logo(而非默认像素占位)。每实例只尝试一次。
  const backfilledIcon = useRef(new Set<string>());
  useEffect(() => {
    const i = inst();
    if (!i || i.icon || backfilledIcon.current.has(i.id)) return;
    // 防 resource 串接竞态:project 由 cfg().source.project_id 异步派生,切换实例时 inst 可能
    // 先于 project 更新——此刻 project() 还是上一个实例的项目,直接拿它的 icon_url 会把别人的
    // logo 写错给当前实例(且 backfilledIcon 锁死再不重试)。只在 project 确实对应当前实例来源
    // (id 对得上)时才补齐,对不上就这轮跳过、等 project 追上后再来。
    const src = cfg()?.source;
    const proj = project();
    if (!src || src.provider !== "modrinth" || !proj || proj.id !== src.project_id) return;
    const url = proj.icon_url;
    if (!url) return;
    backfilledIcon.current.add(i.id);
    void api.backfillInstanceIcon(activeRoot(), i.id, url).then((done) => {
      if (done) {
        refreshInstances();
      }
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instanceList, currentId, cfgData, projectData]);

  const latestUpdate = () => (updates() ?? [])[0];
  const modrinthUrl = () => {
    const pid = cfg()?.source?.project_id;
    return pid ? `https://modrinth.com/project/${pid}` : null;
  };
  // 整合包就地更新:确认弹窗 + 进度。覆盖导入新包到既有实例,存档/配置保留,被移除的模组进回收站。
  const [updateOpen, setUpdateOpen] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [updateProgress, setUpdateProgress] = useState("");

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
      void refreshInstances();
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
  const [browsing, setBrowsing] = useState(false);
  // 删除实例前确认(与实例行的删除确认一致,避免 ⋮ 菜单一点就删)。
  const [confirmDel, setConfirmDel] = useState(false);
  // 导出整合包:选格式弹窗(非空 = 打开)。
  const [exportRow, setExportRow] = useState<InstanceRowData | null>(null);
  const [wikiReindexing, setWikiReindexing] = useState(false);

  // ===== 标签编辑 =====
  // 新标签输入框内容;实例的现有标签直接读 inst().tags(后端单一真相)。
  const [tagInput, setTagInput] = useState("");
  // 提交标签集合到后端(后端会再做去空白/去重),成功后刷新全局列表 + 本页。
  async function saveTags(next: string[]) {
    const i = inst();
    if (!i) return;
    try {
      await api.setInstanceTags(activeRoot(), i.id, next);
      refreshInstances();
    } catch (e) {
      toast({ type: "error", message: t("tags.saveError", { err: String(e) }) });
    }
  }
  function addTag() {
    const i = inst();
    const raw = tagInput.trim();
    if (!i || !raw) return;
    const current = i.tags ?? [];
    if (current.includes(raw)) {
      setTagInput("");
      return;
    }
    void saveTags([...current, raw]);
    setTagInput("");
  }
  function removeTag(tag: string) {
    const i = inst();
    if (!i) return;
    void saveTags((i.tags ?? []).filter((x) => x !== tag));
  }

  // Esc 返回上一页(与详情页导航一致);浏览模式有自己的 Esc,正在输入文本时不抢。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || browsing) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT" || el.isContentEditable))
        return;
      e.preventDefault();
      closeInstance();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [browsing]);

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
      realmRole: i.realm?.role,
      installed: i.installed,
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

  async function askWikiAgent() {
    const i = inst();
    if (!i) return;
    try {
      const root = activeRoot();
      const dir = await api.instanceDir(root, i.id);
      const source = cfg()?.source;
      openAgentChat(
        t("agent.wikiPrompt", {
          name: i.name || i.id,
          version: i.mc_version,
          loader: fmtLoader(i.loader) || t("instance.noLoader"),
        }),
        {
          mode: "instance",
          instance: {
            root,
            modpackId: source?.project_id || i.id,
            instanceId: i.id,
            sourcePaths: [dir],
            mcVersion: i.mc_version,
            loader: i.loader,
          },
        },
      );
    } catch (e) {
      toast({ type: "error", message: t("instance.askWikiFailed", { err: String(e) }) });
    }
  }

  async function rebuildWikiIndex() {
    const i = inst();
    if (!i || wikiReindexing) return;
    setWikiReindexing(true);
    try {
      await api.rebuildInstanceWikiIndex(activeRoot(), i.id);
      toast({ type: "success", message: t("instance.wikiReindexSuccess") });
    } catch (e) {
      toast({ type: "error", message: t("instance.wikiReindexFailed", { err: String(e) }) });
    } finally {
      setWikiReindexing(false);
    }
  }

  async function onMenuAction(value: string) {
    const i = inst();
    const row = toRowData();
    if (!i || !row) return;
    if (value === "open") void openInstanceDir(activeRoot(), i.id);
    else if (value === "copy") await copyCurrent();
    else if (value === "rebuildWiki") await rebuildWikiIndex();
    else if (value === "export") setExportRow(row);
    else if (value === "delete") setConfirmDel(true);
  }

  async function doDelete() {
    const i = inst();
    if (!i) return;
    setConfirmDel(false);
    if (await deleteInstance(activeRoot(), { id: i.id, name: i.name || i.id })) {
      refreshInstances();
      closeInstance();
    }
  }

  return (
    <div className="flex flex-col h-full min-h-0 overflow-hidden">
      {/* 头部:浏览(添加)模式下隐藏,整页让给复用的探索视图。 */}
      {!browsing && (
        <div className="flex flex-col gap-[12px] px-[28px] pt-[14px] pb-[16px] border-b border-titlebar">
          {/* 返回:整行最上方的文字返回,与其它详情页一致。 */}
          <button
            className="self-start inline-flex items-center gap-[4px] bg-transparent border-none text-muted text-[13px] cursor-pointer py-[2px] px-0 transition-colors duration-150 hover:text-fg"
            onClick={closeInstance}
            aria-label={t("instance.back")}
          >
            <svg className="w-[16px] h-[16px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round">
              <path d="m14 6-6 6 6 6" />
            </svg>
            {t("instance.back")}
          </button>

          {(() => {
            const i = inst();
            if (!i)
              return <div className="text-muted text-[14px] py-[8px]">{t("instance.loading")}</div>;
            return (
              <div className="flex items-center gap-[14px]">
                <div className="relative shrink-0 w-[56px] h-[56px] rounded-none overflow-hidden select-none shadow-sunken">
                  <InstanceIcon name={i.name || i.id} icon={i.icon || project()?.icon_url || undefined} />
                  {running.has(i.id) && (
                    <span className="absolute right-[3px] bottom-[3px] w-[12px] h-[12px] rounded-none bg-accent shadow-[0_0_0_2px_var(--bg-window)]" title={t("instance.running")} />
                  )}
                </div>
                <div className="flex-1 min-w-0">
                  <Heading size="section" className="whitespace-nowrap overflow-hidden text-ellipsis" title={i.name || i.id}>
                    {i.name || i.id}
                  </Heading>
                  <div className="mt-[2px] flex items-center gap-[6px] text-[12px] text-sub whitespace-nowrap overflow-hidden text-ellipsis">
                    <span>{loaderLabel()}</span>
                    <span className="text-faint">·</span>
                    <span>{playedLabel()}</span>
                    {project() && (
                      <>
                        <span className="text-faint">·</span>
                        <span className="inline-flex items-center gap-[4px]">
                          <svg className="w-[12px] h-[12px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                            <path d="M12 3v12m0 0 4-4m-4 4-4-4M5 21h14" />
                          </svg>
                          <PixelLabel>{project()!.downloads.toLocaleString()}</PixelLabel>
                        </span>
                        {!!project()!.followers && (
                          <>
                            <span className="text-faint">·</span>
                            <span className="inline-flex items-center gap-[4px]">
                              <svg className="w-[12px] h-[12px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                                <path d="M19 14c1.5-1.5 3-3.3 3-5.5A4.5 4.5 0 0 0 12 5 4.5 4.5 0 0 0 2 8.5c0 2.2 1.5 4 3 5.5l7 7Z" />
                              </svg>
                              <PixelLabel>{project()!.followers.toLocaleString()}</PixelLabel>
                            </span>
                          </>
                        )}
                      </>
                    )}
                  </div>
                  {/* 整合包分类标签:头部点亮品牌信息,概览不再重复。 */}
                  {(project()?.categories?.length ?? 0) > 0 && (
                    <div className="mt-[6px] flex flex-wrap gap-[5px]">
                      {(project()!.categories ?? []).map((c) => (
                        <Tag key={c} className="capitalize">{c}</Tag>
                      ))}
                    </div>
                  )}
                  {/* 整合包更新提示:有更新时给个可点击的熔岩橙凸起芯片,点开确认弹窗就地更新。 */}
                  {latestUpdate() && (
                    <button
                      type="button"
                      className="inline-flex items-center gap-[6px] mt-[7px] h-[24px] pl-[9px] pr-[10px] rounded-none bg-accent text-accent-text text-[11px] font-semibold shadow-raised cursor-pointer transition-[filter] duration-150 hover:brightness-110 active:shadow-pressed"
                      title={t("instance.updateAvailableHint")}
                      onClick={() => setUpdateOpen(true)}
                    >
                      <svg className="w-[12px] h-[12px]" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                        <path d="M12 20v-9m0 0 4 4m-4-4-4 4M4 8a8 8 0 0 1 16 0" />
                      </svg>
                      {t("instance.updateAvailable", { version: latestUpdate()!.version_number })}
                    </button>
                  )}
                  {/* 标签编辑:现有标签可点 ✕ 移除,输入框 + 添加(回车也行)。库页据此分组/筛选。 */}
                  <div className="mt-[8px] flex flex-wrap items-center gap-[6px]">
                    {(i.tags ?? []).map((tag) => (
                      <Chip key={tag} onRemove={() => removeTag(tag)} removeLabel={t("tags.remove", { tag })}>
                        {tag}
                      </Chip>
                    ))}
                    <div className="inline-flex items-center gap-[6px]">
                      <input
                        type="text"
                        value={tagInput}
                        onChange={(e) => setTagInput(e.currentTarget.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") {
                            e.preventDefault();
                            addTag();
                          }
                        }}
                        placeholder={t("tags.addPlaceholder")}
                        className="h-[28px] w-[140px] bg-panel-2 text-fg text-[12px] px-[10px] rounded-none shadow-input border-none outline-none placeholder:text-faint focus:shadow-pressed"
                      />
                      <Button variant="ghost" disabled={!tagInput.trim()} onClick={addTag}>
                        {t("tags.add")}
                      </Button>
                    </div>
                  </div>
                </div>
                <PlayButton
                  running={running.has(i.id)}
                  disabled={launching.has(i.id) || !i.installed}
                  onClick={() => void playInstance(i.id)}
                />
                <Button
                  variant="ghost"
                  className="!h-[38px] !px-[12px] !text-[12px] shrink-0"
                  title={t("instance.askWiki")}
                  onClick={() => void askWikiAgent()}
                >
                  <BookOpen className="w-[15px] h-[15px]" aria-hidden="true" />
                  {t("instance.askWiki")}
                </Button>
                <Menu.Root positioning={{ placement: "bottom-end" }} onSelect={(d: { value: string }) => void onMenuAction(d.value)}>
                  <Menu.Trigger
                    className="inline-flex items-center justify-center w-[38px] h-[38px] border-none bg-panel-3 text-sub rounded-none shadow-raised cursor-pointer transition-[filter,color] duration-[var(--dur)] ease-app hover:brightness-110 hover:text-fg active:shadow-pressed data-[state=open]:shadow-pressed data-[state=open]:text-fg"
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
                    <Menu.Item value="rebuildWiki">
                      {wikiReindexing ? t("instance.wikiReindexing") : t("instance.rebuildWikiIndex")}
                    </Menu.Item>
                    <Menu.Item value="export">{t("instance.exportModpack")}</Menu.Item>
                    <Menu.Separator />
                    <Menu.Item value="delete" danger>
                      {t("instance.deleteInstance")}
                    </Menu.Item>
                  </Menu.Content>
                </Menu.Root>
              </div>
            );
          })()}
        </div>
      )}

      {/* 非领域实例:「分享为领域」入口(块状,置于 tabs 上方)。
          领域实例的同步 / 成员管理已移进 tabs 里的「领域」标签(InstanceManageDialog)。 */}
      {socialOn && !browsing && !inst()?.realm && inst() && (
        <div className="shrink-0 border-b border-titlebar overflow-y-auto max-h-[55vh]">
          <RealmPanel instance={inst()!} onChanged={() => void refreshInstances()} />
        </div>
      )}

      {/* tabs + 内容(复用管理面板的 embedded 模式) */}
      <div className="flex-1 min-h-0 overflow-hidden">
        {inst() && (
          <InstanceManageDialog
            embedded
            open
            instance={inst()!}
            onChanged={() => void refreshInstances()}
            onCopied={(newId: string) => openInstance(newId)}
            onBrowsingChange={setBrowsing}
          />
        )}
      </div>

      <Dialog
        open={confirmDel}
        onClose={() => setConfirmDel(false)}
        label={t("instance.deleteInstance")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] bg-panel shadow-raised rounded-none overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <Heading size="sub" className="text-strong break-words">
            {t("instance.deleteInstanceConfirm", { name: inst()?.name || inst()?.id || "" })}
          </Heading>
          <div className="text-[13px] text-sub leading-[1.6]">
            {t("instance.deleteInstanceBodyDetail")}
          </div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDel(false)}>
              {t("instance.cancel")}
            </Button>
            <Button variant="danger" onClick={() => void doDelete()}>
              {t("instance.delete")}
            </Button>
          </div>
        </div>
      </Dialog>

      <ExportModpackDialog
        open={!!exportRow}
        root={activeRoot()}
        instance={exportRow}
        onClose={() => setExportRow(null)}
      />

      <Dialog
        open={updateOpen}
        onClose={() => !updating && setUpdateOpen(false)}
        label={t("instance.updateTitle")}
        contentClass="w-[400px] max-w-[calc(100vw-48px)] bg-panel shadow-raised rounded-none overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <Heading size="sub" className="text-strong break-words">{t("instance.updateTitle")}</Heading>
          <div className="text-[13px] text-sub leading-[1.6]">
            {t("instance.updateBody", { version: latestUpdate()?.version_number ?? "" })}
          </div>
          {latestUpdate()?.changelog?.trim() && (
            <div className="flex flex-col gap-[6px]">
              <div className="text-[12px] font-semibold text-muted">{t("instance.updateChangelog")}</div>
              <div
                className="md max-h-[200px] overflow-y-auto rounded-none bg-panel-2 shadow-input px-[12px] py-[10px] text-[12px] leading-[1.6] text-sub"
                dangerouslySetInnerHTML={{ __html: renderMarkdown(latestUpdate()!.changelog) }}
              />
            </div>
          )}
          {modrinthUrl() && (
            <a
              href={modrinthUrl()!}
              className="self-start text-[12px] text-accent no-underline hover:underline"
            >
              {t("instance.viewOnModrinth")} →
            </a>
          )}
          {updating && updateProgress && (
            <div className="text-[12px] text-muted font-mono truncate">{updateProgress}</div>
          )}
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" disabled={updating} onClick={() => setUpdateOpen(false)}>
              {t("instance.cancel")}
            </Button>
            <Button variant="primary" disabled={updating} onClick={() => void applyUpdate()}>
              {updating
                ? t("instance.updating")
                : t("instance.updateNow", { version: latestUpdate()?.version_number ?? "" })}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}
