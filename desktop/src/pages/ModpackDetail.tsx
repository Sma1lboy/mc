import { Component, createResource, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { BlockedFilesDialog, Spinner, toast, Lightbox, Panel, Chip, Tag, Heading, PixelLabel, type ModpackHit, type LightboxImage } from "../components";
import { ACCENT_BTN } from "../components/styles";
import { api, onInstallProgress } from "../ipc/api";
import { cached } from "../ipc/cache";
import { activeRoot, refreshInstances } from "../store";
import { t } from "../i18n";
import type { ImportOutcome, ModrinthVersion, ModrinthProject } from "../ipc/types";
import { renderMarkdown } from "../util/markdown";
import "./ModpackDetail.css"; // 残留:.md ... (innerHTML markdown 标记)

/**
 * ModpackDetail —— 整合包详情页(照 Modrinth 项目页):头部信息 + 三个标签页:
 *   - 「简介」:完整介绍正文(markdown 渲染)+ 外部链接,默认页;
 *   - 「画廊」:专门展示项目截图,点击进入全屏灯箱(上一张/下一张/键盘/缩略图条);
 *   - 「版本」:版本列表(类型/MC/loader/发布时间/下载数 + 更新日志 + 安装)。
 * 所有数据都来自 daemon(api.modrinthProject / api.modrinthVersions)。
 */

const typeLabel = (type: string) =>
  (({ release: t("discover.typeRelease"), beta: t("discover.typeBeta"), alpha: t("discover.typeAlpha") }) as Record<string, string>)[type] ?? type;

const loaderLabel = (l: string) =>
  (({ fabric: "Fabric", forge: "Forge", neoforge: "NeoForge", quilt: "Quilt" }) as Record<
    string,
    string
  >)[l] ?? l;

const fmtSize = (n: number | null) =>
  !n ? "" : n >= 1 << 20 ? `${(n / (1 << 20)).toFixed(1)} MB` : `${Math.ceil(n / 1024)} KB`;

const fmtDate = (s: string) => {
  const d = new Date(s);
  return isNaN(d.getTime()) ? s : d.toLocaleDateString();
};

type Tab = "about" | "gallery" | "versions";

const ModpackDetail: Component<{
  hit: ModpackHit;
  onBack: () => void;
  onInstalled?: () => void;
  /** 内容来源平台(modrinth / curseforge);决定走哪个 provider 取版本/安装。缺省 modrinth。 */
  provider?: "modrinth" | "curseforge";
}> = (props) => {
  const provider = () => props.provider ?? "modrinth";
  const [tab, setTab] = createSignal<Tab>("about");
  // 灯箱当前下标(null = 关闭)。画廊图片源自 daemon 的 project().gallery。
  const [lbIndex, setLbIndex] = createSignal<number | null>(null);
  const gallery = (): LightboxImage[] => project()?.gallery ?? [];

  const [project] = createResource(
    () => props.hit.id,
    (id) =>
      cached(`project|${provider()}|${id}`, () => api.modrinthProject(id)).catch((e) => {
        toast({ type: "error", message: t("discover.aboutLoadFailed", { error: String(e) }) });
        return null as ModrinthProject | null;
      }),
  );

  const [versions] = createResource(
    () => [props.hit.id, provider()] as const,
    ([id, prov]) =>
      cached(`versions|${prov}|${id}`, () => api.modrinthVersions(id, prov)).catch((e) => {
        toast({ type: "error", message: t("discover.versionsLoadFailed", { error: String(e) }) });
        return [] as ModrinthVersion[];
      }),
  );

  const [openLog, setOpenLog] = createSignal<Record<string, boolean>>({});
  const [installing, setInstalling] = createSignal<string | null>(null);
  // 安装进度阶段(来自 install://progress);整包动辄数 GB,没有进度像卡死。
  const [progress, setProgress] = createSignal("");
  const offProgress = onInstallProgress((p) => {
    if (!installing()) return;
    setProgress(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage);
  });
  onCleanup(offProgress);
  // 装完后若有需手动下载 / 被跳过的文件,弹窗摊开给用户(而不是只在 toast 里报个数字)。
  const [outcome, setOutcome] = createSignal<ImportOutcome | null>(null);
  // 头部「安装最新版 ▾」的版本下拉是否展开。
  const [menuOpen, setMenuOpen] = createSignal(false);

  // Esc 返回(与列表/灯箱一致);灯箱/结果弹窗打开或安装进行中时不抢 Esc。
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (menuOpen()) { setMenuOpen(false); e.preventDefault(); return; }
      if (lbIndex() !== null || outcome() !== null || installing()) return;
      e.preventDefault();
      props.onBack();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  const vList = (): ModrinthVersion[] => versions() ?? [];

  // 外部链接:源码/问题/Wiki/Discord(用系统浏览器打开)。
  const links = () => {
    const p = project();
    if (!p) return [] as { label: string; url: string }[];
    return (
      [
        { label: t("discover.linkSource"), url: p.source_url },
        { label: t("discover.linkIssues"), url: p.issues_url },
        { label: t("discover.linkWiki"), url: p.wiki_url },
        { label: t("discover.linkDiscord"), url: p.discord_url },
      ].filter((l) => !!l.url) as { label: string; url: string }[]
    );
  };

  async function install(v: ModrinthVersion) {
    if (installing()) return;
    // Modrinth 走 .mrpack:无下载地址即不可装;CurseForge 无 mrpack_url,按 version_id 取档。
    if (provider() === "modrinth" && !v.mrpack_url) {
      toast({ type: "error", message: t("discover.noMrpack") });
      return;
    }
    setInstalling(v.id);
    setProgress(t("discover.preparing"));
    toast({
      type: "info",
      message: t("discover.installStart", { title: props.hit.title, version: v.version_number }),
    });
    try {
      const out = await api.installModpack(activeRoot(), provider(), props.hit.id, v.id, null, props.hit.icon_url ?? null);
      refreshInstances(); // 新建了实例,库 / 侧栏 / 首页统一刷新
      if (out.blocked.length > 0 || out.skipped_optional.length > 0) {
        setOutcome(out); // 弹窗摊开需手动下载 / 被跳过的文件
      } else {
        toast({ type: "success", message: t("discover.installedModpack", { id: out.instance_id }) });
      }
      props.onInstalled?.();
    } catch (e) {
      toast({ type: "error", message: t("discover.installFailed", { error: String(e) }) });
    } finally {
      setInstalling(null);
      setProgress("");
    }
  }

  return (
    <div class="flex flex-col gap-[16px] px-[2px] pt-[4px] pb-[24px] overflow-y-auto">
      <button
        class="self-start bg-transparent border-none text-accent text-[14px] cursor-pointer py-[4px] px-0 rounded-none transition-opacity duration-[var(--dur)] ease-app hover:opacity-70 focus-visible:outline-none"
        onClick={props.onBack}
      >
        {t("discover.back")}
      </button>

      <div class="flex flex-col gap-[12px]">
        <Show when={props.hit.gallery_url}>
          <img
            class="w-full max-h-[240px] object-cover rounded-none shadow-sunken"
            src={props.hit.gallery_url}
            alt=""
            width="960"
            height="540"
          />
        </Show>
        <div class="flex gap-[14px] items-center">
          <Show
            when={props.hit.icon_url}
            fallback={
              <Panel variant="raised" class="w-[72px] h-[72px] object-cover flex-[0_0_auto] flex items-center justify-center font-display text-[32px] text-strong bg-panel-2">
                {(props.hit.title[0] ?? "?").toUpperCase()}
              </Panel>
            }
          >
            <img
              class="w-[72px] h-[72px] rounded-none object-cover flex-[0_0_auto] shadow-sunken"
              src={props.hit.icon_url}
              alt=""
              width="72"
              height="72"
              style="image-rendering:pixelated"
            />
          </Show>
          <div class="min-w-0">
            <Heading as="h1" size="page" class="whitespace-nowrap overflow-hidden text-ellipsis">{props.hit.title}</Heading>
            <div class="mt-[4px] text-[13px] text-sub">
              by {props.hit.author} · ⬇ {props.hit.downloads.toLocaleString()}
              <Show when={project()?.followers}>
                {" · ♥ "}
                {project()!.followers.toLocaleString()}
              </Show>
            </div>
            <div class="mt-[8px] flex flex-wrap gap-[6px]">
              <For each={props.hit.categories}>
                {(c) => <Tag>{c}</Tag>}
              </For>
            </div>
          </div>

          {/* 头部主操作:安装最新版 + 下拉选具体版本(整合包安装即新建实例)。 */}
          <div class="relative ml-auto shrink-0 self-start flex items-stretch gap-[2px]">
            <button
              class="h-[36px] rounded-none bg-accent px-[16px] text-accent-text text-[13px] font-semibold cursor-pointer shadow-raised active:shadow-pressed hover:bg-accent-hover transition-[box-shadow,background-color] duration-[var(--dur)] ease-app disabled:opacity-50 disabled:cursor-default"
              disabled={installing() !== null || vList().length === 0}
              onClick={() => {
                const v = vList()[0];
                if (v) install(v);
              }}
            >
              {installing() ? progress() || t("discover.installing") : t("discover.installLatestVersion")}
            </button>
            <button
              class="h-[36px] w-[32px] grid place-items-center rounded-none bg-accent text-accent-text text-[14px] cursor-pointer shadow-raised active:shadow-pressed hover:bg-accent-hover transition-[box-shadow,background-color] duration-[var(--dur)] ease-app disabled:opacity-50 disabled:cursor-default"
              disabled={installing() !== null || vList().length === 0}
              title={t("discover.chooseVersion")}
              aria-label={t("discover.chooseVersion")}
              onClick={() => setMenuOpen((o) => !o)}
            >
              ▾
            </button>
            <Show when={menuOpen()}>
              <div class="fixed inset-0 z-20" onClick={() => setMenuOpen(false)} />
              <Panel variant="raised" class="absolute right-0 top-[42px] z-30 w-[340px] max-h-[380px] overflow-y-auto bg-panel p-[6px]">
                <div class="px-[8px] py-[6px] text-[12px] text-muted">{t("discover.chooseVersion")}</div>
                <For each={vList()}>
                  {(v) => (
                    <button
                      class="w-full flex items-center justify-between gap-[10px] px-[10px] py-[8px] rounded-none bg-transparent border-none text-left cursor-pointer hover:bg-panel-2 transition-colors duration-[var(--dur)] ease-app disabled:opacity-50 disabled:cursor-default"
                      disabled={installing() !== null}
                      onClick={() => {
                        setMenuOpen(false);
                        install(v);
                      }}
                    >
                      <span class="min-w-0 flex-1 text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">{v.version_number}</span>
                      <span class="shrink-0 text-[11px] text-muted">{typeLabel(v.version_type)} · {fmtDate(v.date_published)}</span>
                    </button>
                  )}
                </For>
              </Panel>
            </Show>
          </div>
        </div>
        <Show when={props.hit.description}>
          <p class="m-0 text-[14px] leading-[1.7] text-sub">{props.hit.description}</p>
        </Show>
      </div>

      {/* ---- 标签页切换 ---- */}
      <div class="flex gap-[6px]">
        <Chip active={tab() === "about"} onClick={() => setTab("about")}>
          {t("discover.tabAbout")}
        </Chip>
        <Chip active={tab() === "versions"} onClick={() => setTab("versions")}>
          {t("discover.tabVersions")}
          <Show when={(versions() ?? []).length}>
            <PixelLabel class="ml-[4px]">{(versions() ?? []).length}</PixelLabel>
          </Show>
        </Chip>
        <Show when={gallery().length}>
          <Chip active={tab() === "gallery"} onClick={() => setTab("gallery")}>
            {t("discover.tabGallery")}
            <PixelLabel class="ml-[4px]">{gallery().length}</PixelLabel>
          </Chip>
        </Show>
      </div>

      {/* ---- 简介 ---- */}
      <Show when={tab() === "about"}>
        <div class="flex flex-col gap-[16px]">
          <Show
            when={!project.loading}
            fallback={
              <div class="p-[28px] text-center text-muted">
                <Spinner />
              </div>
            }
          >
            <Show
              when={project()}
              fallback={<div class="p-[28px] text-center text-muted">{t("discover.noAbout")}</div>}
            >
              {(p) => (
                <>
                  <Show when={links().length}>
                    <div class="flex flex-wrap gap-[8px]">
                      <For each={links()}>
                        {(l) => (
                          <button
                            class="py-[6px] px-[14px] rounded-none bg-panel-3 text-tag text-[13px] cursor-pointer shadow-raised active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
                            onClick={() => shellOpen(l.url)}
                          >
                            {l.label} ↗
                          </button>
                        )}
                      </For>
                    </div>
                  </Show>

                  <Show
                    when={p().body?.trim()}
                    fallback={
                      <div class="p-[28px] text-center text-muted">{t("discover.noAboutBody")}</div>
                    }
                  >
                    {/* renderMarkdown 转义优先,输出仅含白名单标签,innerHTML 安全 */}
                    <div
                      class="md text-[14px] leading-[1.75] text-sub"
                      innerHTML={renderMarkdown(p().body)}
                    />
                  </Show>
                </>
              )}
            </Show>
          </Show>
        </div>
      </Show>

      {/* ---- 画廊:专门展示图片,点击进灯箱 ---- */}
      <Show when={tab() === "gallery"}>
        <div class="grid grid-cols-[repeat(auto-fill,minmax(340px,1fr))] gap-[16px]">
          <For each={gallery()}>
            {(g, i) => (
              <figure class="m-0 flex flex-col gap-[6px]">
                <img
                  class="w-full aspect-[16/9] object-cover rounded-none cursor-zoom-in bg-panel-2 shadow-sunken transition-transform duration-[var(--dur)] ease-app hover:scale-[1.015]"
                  src={g.url}
                  alt={g.title ?? ""}
                  width="960"
                  height="540"
                  loading="lazy"
                  onClick={() => setLbIndex(i())}
                />
                <Show when={g.title}>
                  <figcaption class="text-[12px] text-muted">{g.title}</figcaption>
                </Show>
              </figure>
            )}
          </For>
        </div>
      </Show>

      {/* ---- 版本(TanStack 虚拟化:只渲染可视区 + overscan)---- */}
      <Show when={tab() === "versions"}>
        <div class="flex flex-col">
          <Show
            when={!versions.loading}
            fallback={
              <div class="p-[28px] text-center text-muted">
                <Spinner />
              </div>
            }
          >
            <Show
              when={vList().length > 0}
              fallback={<div class="p-[28px] text-center text-muted">{t("discover.noVersions")}</div>}
            >
              <div class="overflow-y-auto max-h-[calc(100vh-300px)] flex flex-col gap-[6px] px-[2px] -mx-[2px]">
                <For each={vList()}>
                  {(v) => (
                    <Panel variant="sunken" class="flex items-start gap-[12px] bg-panel py-[10px] px-[12px]">
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-[8px]">
                        <span class="text-[14px] font-bold text-strong">{v.version_number}</span>
                        <span
                          class="text-[11px] py-[2px] px-[7px] rounded-none bg-panel-2 text-tag data-[type=release]:text-[#7bbf5a] data-[type=beta]:text-[#d8a23c] data-[type=alpha]:text-[#d97a4a]"
                          data-type={v.version_type}
                        >
                          {typeLabel(v.version_type)}
                        </span>
                      </div>
                      <div class="mt-[4px] text-[12px] text-muted">
                        {v.game_versions.slice(0, 5).join(", ")}
                        <Show when={v.loaders.length}>
                          {" · "}
                          {v.loaders.map(loaderLabel).join(" / ")}
                        </Show>
                        {" · "}
                        {fmtDate(v.date_published)} · ⬇ {v.downloads.toLocaleString()}
                        <Show when={v.file_size}>{" · " + fmtSize(v.file_size)}</Show>
                      </div>
                      <Show when={v.changelog?.trim()}>
                        <button
                          class="mt-[8px] bg-transparent border-none p-0 text-[12px] text-accent cursor-pointer hover:underline"
                          onClick={() => setOpenLog((o) => ({ ...o, [v.id]: !o[v.id] }))}
                        >
                          {openLog()[v.id] ? t("discover.collapseChangelog") : t("discover.changelog")}
                        </button>
                        <Show when={openLog()[v.id]}>
                          <div
                            class="md mt-[8px] mb-0 mx-0 py-[6px] px-[12px] max-h-[260px] overflow-y-auto [word-break:break-word] text-[12px] text-sub bg-window shadow-input"
                            innerHTML={renderMarkdown(v.changelog)}
                          />
                        </Show>
                      </Show>
                    </div>
                            <button
                              class={`flex-[0_0_auto] ${ACCENT_BTN}`}
                              disabled={(provider() === "modrinth" && !v.mrpack_url) || installing() !== null}
                              onClick={() => install(v)}
                            >
                              {installing() === v.id ? (progress() || t("discover.installing")) : t("discover.installThisVersion")}
                            </button>
                    </Panel>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </div>
      </Show>

      {/* ---- 全屏灯箱(画廊点击进入)---- */}
      <Show when={lbIndex() !== null}>
        <Lightbox
          images={gallery()}
          index={lbIndex()!}
          onIndex={setLbIndex}
          onClose={() => setLbIndex(null)}
        />
      </Show>

      <Show when={outcome()}>
        {(o) => (
          <BlockedFilesDialog
            instanceId={o().instance_id}
            blocked={o().blocked}
            skipped={o().skipped_optional}
            onClose={() => setOutcome(null)}
          />
        )}
      </Show>
    </div>
  );
};

export default ModpackDetail;
