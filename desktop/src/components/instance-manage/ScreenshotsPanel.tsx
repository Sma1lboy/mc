import { useEffect, useRef, useState } from "react";
import type { MouseEvent as ReactMouseEvent } from "react";
import Lightbox from "../Lightbox";
import { ErrorState } from "../ErrorState";
import { Spinner } from "../Spinner";
import { toast } from "../Toast";
import { api } from "../../ipc/api";
import { activeRoot } from "../../store";
import { useAsync } from "../../util/useAsync";
import { t } from "../../i18n";
import type { InstanceSummary, ScreenshotInfo } from "../../ipc/types";
import { LABEL, OPEN_BTN } from "./shared";
import { openInstanceSubdir } from "../../util/instanceActions";

const SCREENSHOT_CAP = 60;

/**
 * ScreenshotTile —— 单张截图缩略图。用 IntersectionObserver 懒加载:滚动到视口附近才
 * 通过 read_screenshot 取该张的 data URL,避免把目录里所有大图一次性读进内存。
 */
function ScreenshotTile(props: {
  info: ScreenshotInfo;
  url?: string;
  failed?: boolean;
  onVisible: () => void;
  onOpen: () => void;
  onRetry: () => void;
  onDelete: (e: ReactMouseEvent) => void;
}) {
  const elRef = useRef<HTMLDivElement>(null);
  // onVisible 通过 ref 读最新闭包(观察器只装一次,回调却按当前渲染取值)。
  const onVisibleRef = useRef(props.onVisible);
  onVisibleRef.current = props.onVisible;
  useEffect(() => {
    const el = elRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          onVisibleRef.current();
          io.disconnect();
        }
      },
      { rootMargin: "120px" },
    );
    io.observe(el);
    return () => io.disconnect();
  }, []);

  return (
    <div
      ref={elRef}
      className="group relative aspect-video rounded-none overflow-hidden bg-panel-2 cursor-pointer"
      onClick={props.onOpen}
    >
      {props.url ? (
        <img src={props.url} alt={props.info.file_name} width="320" height="180" className="w-full h-full object-cover" />
      ) : props.failed ? (
        // 读图失败:给可重试的占位,而不是永远转圈。
        <button
          className="w-full h-full grid place-items-center gap-[2px] text-[11px] text-muted bg-panel-2 cursor-pointer hover:text-fg"
          onClick={(e) => {
            e.stopPropagation();
            props.onRetry();
          }}
          title={t("instance.reload")}
        >
          <span>{t("instance.loadFailed")}</span>
          <span className="text-[10px] underline">{t("instance.clickRetry")}</span>
        </button>
      ) : (
        <div className="w-full h-full grid place-items-center">
          <Spinner size={16} />
        </div>
      )}
      <button
        className="absolute top-[4px] right-[4px] opacity-0 group-hover:opacity-100 transition-opacity duration-150 text-[11px] text-white px-[6px] py-[2px] rounded-none bg-[rgba(0,0,0,0.55)] hover:bg-danger"
        onClick={props.onDelete}
      >
        {t("instance.delete")}
      </button>
    </div>
  );
}

/**
 * ScreenshotsPanel —— 实例截图栅格:懒加载缩略图、点开进灯箱、悬停删除。
 * 列表只取元数据,图片字节按需 read_screenshot;最多展示 SCREENSHOT_CAP 张(更多时提示)。
 */
export function ScreenshotsPanel(props: { instance: InstanceSummary }) {
  const { data: shots, loading: shotsLoading, error: shotsError, refetch } = useAsync<ScreenshotInfo[]>(
    () => api.instanceScreenshots(activeRoot(), props.instance.id),
    [props.instance.id],
  );
  const capped = (shots ?? []).slice(0, SCREENSHOT_CAP);
  const [urls, setUrls] = useState<Record<string, string>>({});
  const [failed, setFailed] = useState<Record<string, boolean>>({});
  const [lightbox, setLightbox] = useState<number | null>(null);
  // loadUrl 的去重要读最新已加载 urls(否则拿到旧闭包会重复取同一张)。
  const urlsRef = useRef(urls);
  urlsRef.current = urls;

  async function loadUrl(fileName: string) {
    if (urlsRef.current[fileName]) return;
    setFailed((f) => ({ ...f, [fileName]: false }));
    try {
      const u = await api.readScreenshot(activeRoot(), props.instance.id, fileName);
      setUrls((m) => ({ ...m, [fileName]: u }));
    } catch {
      // 单张读失败不致命:标记失败态,渲染可重试占位,不让缩略图永远转圈。
      setFailed((f) => ({ ...f, [fileName]: true }));
    }
  }

  async function remove(s: ScreenshotInfo, e: ReactMouseEvent) {
    e.stopPropagation(); // 别触发打开灯箱。
    try {
      await api.deleteScreenshot(activeRoot(), props.instance.id, s.file_name);
      toast({ type: "success", message: t("instance.deletedScreenshot") });
      refetch();
    } catch (err) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(err) }) });
    }
  }

  const lightboxImages = capped.map((s) => ({ url: urls[s.file_name] ?? "", title: s.file_name }));

  // 打开/切换灯箱时确保目标图及左右相邻图已加载(缩略图可能还没滚动到、未触发懒加载),
  // 避免主图/缩略图条出现空白或裂图。
  function openLightbox(i: number) {
    for (const j of [i, i - 1, i + 1]) {
      const f = capped[j]?.file_name;
      if (f) void loadUrl(f);
    }
    setLightbox(i);
  }

  return (
    <div className="flex flex-col gap-[8px]">
      <div className="flex items-center justify-between">
        <div className={LABEL}>{t("instance.screenshots")}</div>
        <button
          className={OPEN_BTN}
          onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "screenshots")}
        >
          {t("instance.openDir")}
        </button>
      </div>

      {(shots ?? []).length > SCREENSHOT_CAP && (
        <div className="text-[11px] text-muted">
          {t("instance.screenshotCapNote", { total: (shots ?? []).length, cap: SCREENSHOT_CAP })}
        </div>
      )}

      {shotsLoading ? (
        <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
          <Spinner size={16} /> {t("instance.scanningScreenshots")}
        </div>
      ) : capped.length > 0 ? (
        <div className="grid grid-cols-3 gap-[8px]">
          {capped.map((s, i) => (
            <ScreenshotTile
              key={s.file_name}
              info={s}
              url={urls[s.file_name]}
              failed={failed[s.file_name]}
              onVisible={() => loadUrl(s.file_name)}
              onOpen={() => openLightbox(i)}
              onRetry={() => loadUrl(s.file_name)}
              onDelete={(e) => remove(s, e)}
            />
          ))}
        </div>
      ) : shotsError ? (
        <ErrorState compact message={t("instance.screenshotLoadError")} onRetry={() => void refetch()} />
      ) : (
        <div className="text-muted text-[13px] py-[12px]">{t("instance.noScreenshots")}</div>
      )}

      {lightbox !== null && (
        <Lightbox
          images={lightboxImages}
          index={lightbox}
          onIndex={openLightbox}
          onClose={() => setLightbox(null)}
        />
      )}
    </div>
  );
}
