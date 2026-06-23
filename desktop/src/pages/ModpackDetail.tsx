import { Component, createResource, createSignal, For, Show, onMount, onCleanup } from "solid-js";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { BlockedFilesDialog, Spinner, toast, Lightbox, type ModpackHit, type LightboxImage } from "../components";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot } from "../store";
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

const typeLabel = (t: string) =>
  (({ release: "正式版", beta: "测试版", alpha: "内测版" }) as Record<string, string>)[t] ?? t;

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
}> = (props) => {
  const [tab, setTab] = createSignal<Tab>("about");
  // 灯箱当前下标(null = 关闭)。画廊图片源自 daemon 的 project().gallery。
  const [lbIndex, setLbIndex] = createSignal<number | null>(null);
  const gallery = (): LightboxImage[] => project()?.gallery ?? [];

  const [project] = createResource(
    () => props.hit.id,
    (id) =>
      api.modrinthProject(id).catch((e) => {
        toast({ type: "error", message: `简介加载失败:${e}` });
        return null as ModrinthProject | null;
      }),
  );

  const [versions] = createResource(
    () => props.hit.id,
    (id) =>
      api.modrinthVersions(id).catch((e) => {
        toast({ type: "error", message: `版本列表加载失败:${e}` });
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

  // Esc 返回(与列表/灯箱一致);灯箱/结果弹窗打开或安装进行中时不抢 Esc。
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (lbIndex() !== null || outcome() !== null || installing()) return;
      e.preventDefault();
      props.onBack();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  // 版本列表虚拟化(热门整合包可达数百版本,只渲染可视区 + overscan)。
  // 行高可变(更新日志可展开),靠 measureElement(ResizeObserver)动态测量。
  const vList = (): ModrinthVersion[] => versions() ?? [];
  const [versionsScrollEl, setVersionsScrollEl] = createSignal<HTMLDivElement | null>(null);
  const versionVirtualizer = createVirtualizer({
    get count() {
      return vList().length;
    },
    getScrollElement: () => versionsScrollEl(),
    estimateSize: () => 92,
    overscan: 6,
    getItemKey: (i) => vList()[i]?.id ?? i,
  });

  // 外部链接:源码/问题/Wiki/Discord(用系统浏览器打开)。
  const links = () => {
    const p = project();
    if (!p) return [] as { label: string; url: string }[];
    return (
      [
        { label: "源码", url: p.source_url },
        { label: "问题反馈", url: p.issues_url },
        { label: "Wiki", url: p.wiki_url },
        { label: "Discord", url: p.discord_url },
      ].filter((l) => !!l.url) as { label: string; url: string }[]
    );
  };

  async function install(v: ModrinthVersion) {
    if (installing()) return;
    if (!v.mrpack_url) {
      toast({ type: "error", message: "该版本没有可安装的 .mrpack 文件" });
      return;
    }
    setInstalling(v.id);
    setProgress("准备…");
    toast({
      type: "info",
      message: `开始安装「${props.hit.title} ${v.version_number}」…首次会下载原版与依赖,可能需要几分钟`,
    });
    try {
      const out = await api.installModpackUrl(activeRoot(), v.mrpack_url, null);
      if (out.blocked.length > 0 || out.skipped_optional.length > 0) {
        setOutcome(out); // 弹窗摊开需手动下载 / 被跳过的文件
      } else {
        toast({ type: "success", message: `已安装「${out.instance_id}」,去启动页选择它即可开玩` });
      }
      props.onInstalled?.();
    } catch (e) {
      toast({ type: "error", message: `安装失败:${e}` });
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
        ← 返回
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
          简介
        </button>
        <button
          class="relative bg-transparent border-none py-[8px] px-[16px] text-[14px] font-semibold cursor-pointer border-b-2 border-b-transparent mb-[-1px] transition-colors duration-[var(--mo-dur-fast)] ease-emph"
          classList={{
            "text-n-6 hover:text-n-8": tab() !== "versions",
            "text-a-6 !border-b-a-5": tab() === "versions",
          }}
          onClick={() => setTab("versions")}
        >
          版本
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
            画廊
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
              fallback={<div class="p-[28px] text-center text-n-6">没有简介信息</div>}
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
                      <div class="p-[28px] text-center text-n-6">作者没有填写详细介绍。</div>
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
              fallback={<div class="p-[28px] text-center text-n-6">没有可用版本</div>}
            >
              <div
                ref={setVersionsScrollEl}
                class="overflow-y-auto max-h-[calc(100vh-300px)] px-[2px] -mx-[2px]"
              >
                <div
                  class="relative w-full"
                  style={{ height: `${versionVirtualizer.getTotalSize()}px` }}
                >
                  <For each={versionVirtualizer.getVirtualItems()}>
                    {(vi) => {
                      const v = vList()[vi.index];
                      return (
                        <div
                          data-index={vi.index}
                          ref={(el) => versionVirtualizer.measureElement(el)}
                          class="absolute top-0 left-0 w-full"
                          style={{ transform: `translateY(${vi.start}px)` }}
                        >
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
                          {openLog()[v.id] ? "收起更新日志" : "更新日志"}
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
                              disabled={!v.mrpack_url || installing() !== null}
                              onClick={() => install(v)}
                            >
                              {installing() === v.id ? (progress() || "安装中…") : "安装此版本"}
                            </button>
                          </div>
                        </div>
                      );
                    }}
                  </For>
                </div>
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
