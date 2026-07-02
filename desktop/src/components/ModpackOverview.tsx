import { useState } from "react";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner } from "./Spinner";
import { ErrorState } from "./ErrorState";
import Lightbox from "./Lightbox";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { cached } from "../ipc/cache";
import { useAsync } from "../util/useAsync";
import { t, useLang } from "../i18n";
import { renderMarkdown } from "../util/markdown";
import type { LightboxImage } from "./Lightbox";
import "../pages/ModpackDetail.css"; // .md markdown 标记样式

/**
 * ModpackOverview —— 整合包来源实例「概览」标签:只读展示该整合包的平台详情
 * (图标/简介/分类/链接/画廊/正文;Modrinth / CurseForge 同一份渲染模型)。
 * 不含安装流程(实例已装),区别于 ModpackDetail。
 */
export function ModpackOverview(props: { projectId: string; provider?: string }): React.ReactElement {
  useLang();
  const provider = props.provider ?? "modrinth";
  const { data: project, loading, refetch } = useAsync(
    () =>
      cached(`project|${provider}|${props.projectId}`, () => api.modrinthProject(props.projectId, provider)).catch((e) => {
        toast({ type: "error", message: t("discover.modpackDetailLoadFailed", { error: String(e) }) });
        return null;
      }),
    [props.projectId, provider],
  );
  const [lb, setLb] = useState<number | null>(null);
  const gallery: LightboxImage[] = project?.gallery ?? [];
  const links = (() => {
    if (!project) return [] as { label: string; url: string }[];
    return [
      { label: t("discover.linkSource"), url: project.source_url },
      { label: t("discover.linkIssues"), url: project.issues_url },
      { label: t("discover.linkWiki"), url: project.wiki_url },
      { label: t("discover.linkDiscord"), url: project.discord_url },
    ].filter((l) => !!l.url) as { label: string; url: string }[];
  })();

  if (loading) {
    return (
      <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
        <Spinner size={16} /> {t("discover.loadingModpackDetail")}
      </div>
    );
  }

  if (!project) {
    return <ErrorState message={t("discover.modpackDetailUnavailable")} onRetry={() => refetch()} />;
  }

  return (
    <div className="flex flex-col gap-[16px]">
      {/* 品牌信息(图标/名称/下载量/分类)已上移到实例头部,此处不再重复;概览只留简介/链接/画廊/正文。 */}
      {project.description && <p className="m-0 text-[13px] leading-[1.7] text-sub">{project.description}</p>}

      {links.length > 0 && (
        <div className="flex flex-wrap gap-[6px]">
          {links.map((l) => (
            <button
              key={l.url}
              className="h-[28px] px-[12px] rounded-none bg-panel-3 text-tag text-[12px] cursor-pointer shadow-raised active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
              onClick={() => shellOpen(l.url)}
            >
              {l.label} ↗
            </button>
          ))}
        </div>
      )}

      {gallery.length > 0 && (
        <div className="grid grid-cols-3 gap-[8px]">
          {gallery.map((g, i) => (
            <img
              key={g.url}
              className="w-full aspect-video object-cover rounded-none cursor-zoom-in bg-panel-2 shadow-sunken"
              src={g.url}
              alt={g.title ?? ""}
              width="320"
              height="180"
              loading="lazy"
              onClick={() => setLb(i)}
            />
          ))}
        </div>
      )}

      {project.body?.trim() && (
        <div className="md text-[13px] leading-[1.7] text-sub" dangerouslySetInnerHTML={{ __html: renderMarkdown(project.body) }} />
      )}

      {lb !== null && <Lightbox images={gallery} index={lb} onIndex={setLb} onClose={() => setLb(null)} />}
    </div>
  );
}

export default ModpackOverview;
