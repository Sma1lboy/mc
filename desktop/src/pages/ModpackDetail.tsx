import { Component, createResource, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { BlockedFilesDialog, Spinner, toast, Lightbox, type ModpackHit, type LightboxImage } from "../components";
import { api, onInstallProgress } from "../ipc/api";
import { cached } from "../ipc/cache";
import { activeRoot } from "../store";
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
        class="self-start bg-transparent border-none text-a-6 text-[14px] cursor-pointer py-[4px] px-0 rounded-xs transition-opacity duration-[var(--mo-dur-fast)] ease-emph hover:opacity-70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
        onClick={props.onBack}
      >
        {t("discover.back")}
      </button>

      <div class="flex flex-col gap-[12px]">
        <Show when={props.hit.gallery_url}>
          <img
            class="w-full max-h-[240px] object-cover rounded-card"
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
              <div class="w-[72px] h-[72px] rounded-[14px] object-cover flex-[0_0_auto] flex items-center justify-center text-[32px] font-bold text-white bg-[linear-gradient(135deg,var(--a-5,#1370f3),var(--a-7,#4890f5))]">
                {(props.hit.title[0] ?? "?").toUpperCase()}
              </div>
            }
          >
            <img
              class="w-[72px] h-[72px] rounded-[14px] object-cover flex-[0_0_auto]"
              src={props.hit.icon_url}
              alt=""
              width="72"
              height="72"
            />
          </Show>
          <div class="min-w-0">
            <h1 class="m-0 text-[24px] font-extrabold text-n-8 whitespace-nowrap overflow-hidden text-ellipsis">{props.hit.title}</h1>
            <div class="mt-[4px] text-[13px] text-n-6">
              by {props.hit.author} · ⬇ {props.hit.downloads.toLocaleString()}
              <Show when={project()?.followers}>
                {" · ♥ "}
                {project()!.followers.toLocaleString()}
              </Show>
            </div>
            <div class="mt-[8px] flex flex-wrap gap-[6px]">
              <For each={props.hit.categories}>
                {(c) => (
                  <span class="text-[11px] py-[2px] px-[8px] rounded-full bg-a-1 text-a-6">{c}</span>
                )}
              </For>
            </div>
          </div>

          {/* 头部主操作:安装最新版 + 下拉选具体版本(整合包安装即新建实例)。 */}
          <div class="relative ml-auto shrink-0 self-start flex items-stretch">
            <button
              class="h-[36px] rounded-l-ctl border-none bg-a-5 px-[16px] text-white text-[13px] font-semibold cursor-pointer transition-opacity duration-150 hover:opacity-90 disabled:opacity-50 disabled:cursor-default"
              disabled={installing() !== null || vList().length === 0}
              onClick={() => {
                const v = vList()[0];
                if (v) install(v);
              }}
            >
              {installing() ? progress() || t("discover.installing") : t("discover.installLatestVersion")}
            </button>
            <button
              class="h-[36px] w-[30px] rounded-r-ctl border-none bg-a-5 text-white text-[14px] cursor-pointer transition-opacity duration-150 hover:opacity-90 disabled:opacity-50 disabled:cursor-default"
              style={{ "border-left": "1px solid rgba(255,255,255,0.22)" }}
              disabled={installing() !== null || vList().length === 0}
              title={t("discover.chooseVersion")}
              aria-label={t("discover.chooseVersion")}
              onClick={() => setMenuOpen((o) => !o)}
            >
              ▾
            </button>
            <Show when={menuOpen()}>
              <div class="fixed inset-0 z-20" onClick={() => setMenuOpen(false)} />
              <div class="absolute right-0 top-[42px] z-30 w-[340px] max-h-[380px] overflow-y-auto glass-panel rounded-card border border-glass-border p-[6px]">
                <div class="px-[8px] py-[6px] text-[12px] text-n-6">{t("discover.chooseVersion")}</div>
                <For each={vList()}>
                  {(v) => (
                    <button
                      class="w-full flex items-center justify-between gap-[10px] px-[10px] py-[8px] rounded-ctl bg-transparent border-none text-left cursor-pointer transition-colors duration-150 hover:bg-glass-hover disabled:opacity-50 disabled:cursor-default"
                      disabled={installing() !== null}
                      onClick={() => {
                        setMenuOpen(false);
                        install(v);
                      }}
                    >
                      <span class="min-w-0 flex-1 text-[13px] text-n-8 whitespace-nowrap overflow-hidden text-ellipsis">{v.version_number}</span>
                      <span class="shrink-0 text-[11px] text-n-6">{typeLabel(v.version_type)} · {fmtDate(v.date_published)}</span>
                    </button>
                  )}
                </For>
              </div>
            </Show>
          </div>
        </div>
        <Show when={props.hit.description}>
          <p class="m-0 text-[14px] leading-[1.7] text-n-7">{props.hit.description}</p>
        </Show>
      </div>

      {/* ---- 标签页切换 ---- */}
      <div class="flex gap-[4px] border-b border-b-n-3">
        <button
          class="relative bg-transparent border-none py-[8px] px-[16px] text-[14px] font-semibold cursor-pointer border-b-2 border-b-transparent mb-[-1px] transition-colors duration-[var(--mo-dur-fast)] ease-emph"
          classList={{
            "text-n-6 hover:text-n-8": tab() !== "about",
            "text-a-6 !border-b-a-5": tab() === "about",
          }}
          onClick={() => setTab("about")}
        >
          {t("discover.tabAbout")}
        </button>
        <button
          class="relative bg-transparent border-none py-[8px] px-[16px] text-[14px] font-semibold cursor-pointer border-b-2 border-b-transparent mb-[-1px] transition-colors duration-[var(--mo-dur-fast)] ease-emph"
          classList={{
            "text-n-6 hover:text-n-8": tab() !== "versions",
            "text-a-6 !border-b-a-5": tab() === "versions",
          }}
          onClick={() => setTab("versions")}
        >
          {t("discover.tabVersions")}
          <Show when={(versions() ?? []).length}>
            <span class="ml-[6px] text-[11px] font-semibold px-[6px] rounded-full bg-glass-card text-n-6">
              {(versions() ?? []).length}
            </span>
          </Show>
        </button>
        <Show when={gallery().length}>
          <button
            class="relative bg-transparent border-none py-[8px] px-[16px] text-[14px] font-semibold cursor-pointer border-b-2 border-b-transparent mb-[-1px] transition-colors duration-[var(--mo-dur-fast)] ease-emph"
            classList={{
              "text-n-6 hover:text-n-8": tab() !== "gallery",
              "text-a-6 !border-b-a-5": tab() === "gallery",
            }}
            onClick={() => setTab("gallery")}
          >
            {t("discover.tabGallery")}
            <span class="ml-[6px] text-[11px] font-semibold px-[6px] rounded-full bg-glass-card text-n-6">
              {gallery().length}
            </span>
          </button>
        </Show>
      </div>

      {/* ---- 简介 ---- */}
      <Show when={tab() === "about"}>
        <div class="flex flex-col gap-[16px]">
          <Show
            when={!project.loading}
            fallback={
              <div class="p-[28px] text-center text-n-6">
                <Spinner />
              </div>
            }
          >
            <Show
              when={project()}
              fallback={<div class="p-[28px] text-center text-n-6">{t("discover.noAbout")}</div>}
            >
              {(p) => (
                <>
                  <Show when={links().length}>
                    <div class="flex flex-wrap gap-[8px]">
                      <For each={links()}>
                        {(l) => (
                          <button
                            class="py-[6px] px-[14px] border border-glass-border rounded-[6px] bg-glass-card text-a-6 text-[13px] cursor-pointer transition-colors duration-[var(--mo-dur-fast)] ease-emph hover:bg-a-1"
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
                      <div class="p-[28px] text-center text-n-6">{t("discover.noAboutBody")}</div>
                    }
                  >
                    {/* renderMarkdown 转义优先,输出仅含白名单标签,innerHTML 安全 */}
                    <div
                      class="md text-[14px] leading-[1.75] text-n-7"
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
                  class="w-full aspect-[16/9] object-cover rounded-[8px] cursor-zoom-in bg-glass-card transition-transform duration-[var(--mo-dur-fast)] ease-emph hover:scale-[1.015]"
                  src={g.url}
                  alt={g.title ?? ""}
                  width="960"
                  height="540"
                  loading="lazy"
                  onClick={() => setLbIndex(i())}
                />
                <Show when={g.title}>
                  <figcaption class="text-[12px] text-n-6">{g.title}</figcaption>
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
              <div class="p-[28px] text-center text-n-6">
                <Spinner />
              </div>
            }
          >
            <Show
              when={vList().length > 0}
              fallback={<div class="p-[28px] text-center text-n-6">{t("discover.noVersions")}</div>}
            >
              <div class="overflow-y-auto max-h-[calc(100vh-300px)] px-[2px] -mx-[2px]">
                <For each={vList()}>
                  {(v) => (
                    <div class="flex items-start gap-[12px] py-[12px] px-[2px] border-b border-b-glass-divider">
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-[8px]">
                        <span class="text-[14px] font-bold text-n-8">{v.version_number}</span>
                        <span
                          class="text-[11px] py-[1px] px-[7px] rounded-[3px] bg-glass-card text-n-6 data-[type=release]:bg-[rgba(40,167,69,0.14)] data-[type=release]:text-[#1f9d4d] data-[type=beta]:bg-[rgba(255,159,10,0.16)] data-[type=beta]:text-[#c77800] data-[type=alpha]:bg-[rgba(255,59,48,0.14)] data-[type=alpha]:text-[#d23b30]"
                          data-type={v.version_type}
                        >
                          {typeLabel(v.version_type)}
                        </span>
                      </div>
                      <div class="mt-[4px] text-[12px] text-n-6">
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
                          class="mt-[8px] bg-transparent border-none p-0 text-[12px] text-a-6 cursor-pointer hover:underline"
                          onClick={() => setOpenLog((o) => ({ ...o, [v.id]: !o[v.id] }))}
                        >
                          {openLog()[v.id] ? t("discover.collapseChangelog") : t("discover.changelog")}
                        </button>
                        <Show when={openLog()[v.id]}>
                          <div
                            class="md mt-[8px] mb-0 mx-0 py-[6px] px-[12px] max-h-[260px] overflow-y-auto [word-break:break-word] text-[12px] text-n-7 bg-n-1 rounded-[6px]"
                            innerHTML={renderMarkdown(v.changelog)}
                          />
                        </Show>
                      </Show>
                    </div>
                            <button
                              class="flex-[0_0_auto] py-[8px] px-[16px] border-none rounded-ctl bg-a-4 text-white text-[13px] font-semibold cursor-pointer transition-[background-color,opacity] duration-[var(--mo-dur-fast)] ease-emph hover:not-disabled:bg-a-5 disabled:opacity-50 disabled:cursor-default focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5/40"
                              disabled={(provider() === "modrinth" && !v.mrpack_url) || installing() !== null}
                              onClick={() => install(v)}
                            >
                              {installing() === v.id ? (progress() || t("discover.installing")) : t("discover.installThisVersion")}
                            </button>
                    </div>
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
