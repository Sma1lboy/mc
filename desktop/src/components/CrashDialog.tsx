import { useState } from "react";
import { Dialog } from "./Dialog";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { useAppStore, setCrashReport, type CrashReport } from "../store";
import { api } from "../ipc/api";
import { t, useLang } from "../i18n";

/** 把崩溃类别 slug 映射到本地化标签;空 / 未知回退到「未知错误」。 */
function categoryLabel(slug: string | null): string {
  return t(`crash.cat.${slug ?? "unknown"}`);
}

/** 实例版本一行:`1.20.1 · fabric 0.15.0`(无加载器版本时省略)。 */
function versionLine(r: CrashReport): string {
  const parts: string[] = [];
  if (r.mcVersion) parts.push(r.mcVersion);
  if (r.loader && r.loader !== "vanilla") {
    parts.push(r.loaderVersion ? `${r.loader} ${r.loaderVersion}` : r.loader);
  }
  return parts.join(" · ");
}

/** 组装「复制诊断」用的纯文本报告。 */
function buildReport(r: CrashReport): string {
  const lines: string[] = [];
  lines.push(t("crash.report.header"));
  lines.push(`${t("crash.report.instance")}: ${r.name}`);
  const ver = versionLine(r);
  if (ver) lines.push(`${t("crash.report.version")}: ${ver}`);
  lines.push(`${t("crash.report.exitCode")}: ${r.code ?? "—"}`);
  lines.push(`${t("crash.report.category")}: ${categoryLabel(r.category)}`);
  if (r.reason) lines.push(`${t("crash.report.reason")}: ${r.reason}`);
  if (r.suggestions.length > 0) {
    lines.push(`${t("crash.report.suggestions")}:`);
    for (const s of r.suggestions) lines.push(`  - ${s}`);
  }
  if (r.matched) lines.push(`${t("crash.report.evidence")}: ${r.matched}`);
  if (r.logTail) {
    lines.push(`--- ${t("crash.report.logTail")} ---`);
    lines.push(r.logTail);
  }
  return lines.join("\n");
}

/**
 * CrashDialog —— 游戏异常退出时的崩溃报告弹窗(全局单实例,挂在 App 根部)。
 *
 * 由 store.crashReport 信号驱动:后端 game://exit 带回诊断结果后 store 组装报告并 set,
 * 这里据此展示可读摘要 + 类别 + 建议 + 关键日志 + 可折叠的日志尾部,并提供「复制诊断」与
 * 「打开日志目录」。正常退出不会 set,故不会弹出。
 */
export function CrashDialog() {
  useLang();
  const [showLog, setShowLog] = useState(false);
  const rep = useAppStore((s) => s.crashReport);

  const close = () => {
    setCrashReport(null);
    setShowLog(false);
  };

  const copy = async () => {
    if (!rep) return;
    try {
      await navigator.clipboard.writeText(buildReport(rep));
      toast({ type: "success", message: t("crash.copied") });
    } catch (e) {
      toast({ type: "error", message: t("crash.copyFailed", { error: String(e) }) });
    }
  };

  const openLogs = async () => {
    try {
      const dir = await api.openLogsDir();
      await api.revealPath(dir);
    } catch {
      /* 打开失败时静默:复制诊断仍可用 */
    }
  };

  if (!rep) return null;

  return (
    <Dialog
      open
      onClose={close}
      label={t("crash.title")}
      contentClass="w-[560px] max-w-[calc(100vw-48px)]"
    >
      <div className="flex max-h-[80vh] flex-col">
        {/* 头部:标题 + 副标题 */}
        <div className="flex flex-col gap-[6px] border-b border-titlebar px-[20px] pb-[16px] pt-[18px]">
          <div className="flex items-center gap-[8px]">
            <Heading size="sub">{t("crash.title")}</Heading>
            <span className="rounded-none border border-danger/40 bg-danger-soft px-[8px] py-[2px] text-[11px] font-semibold text-danger-text">
              {categoryLabel(rep.category)}
            </span>
          </div>
          <div className="text-[12px] leading-[1.7] text-sub">{t("crash.subtitle")}</div>
        </div>

        {/* 主体:可滚动 */}
        <div className="flex min-h-0 flex-1 flex-col gap-[14px] overflow-auto px-[20px] py-[16px]">
          {/* 元信息网格 */}
          <div className="grid grid-cols-[auto_1fr] gap-x-[16px] gap-y-[6px] text-[13px]">
            <span className="text-muted">{t("crash.instance")}</span>
            <span className="min-w-0 truncate text-fg">{rep.name}</span>
            {versionLine(rep) && (
              <>
                <span className="text-muted">{t("crash.version")}</span>
                <span className="min-w-0 truncate text-fg">{versionLine(rep)}</span>
              </>
            )}
            <span className="text-muted">{t("crash.exitCode")}</span>
            <span className="text-fg">{rep.code ?? "—"}</span>
          </div>

          {/* 原因 */}
          {rep.reason && (
            <div className="flex flex-col gap-[4px]">
              <div className="text-[11px] font-semibold uppercase tracking-wide text-muted">
                {t("crash.reason")}
              </div>
              <div className="text-[13px] leading-[1.7] text-fg">{rep.reason}</div>
            </div>
          )}

          {/* 建议 */}
          {rep.suggestions.length > 0 && (
            <div className="flex flex-col gap-[6px]">
              <div className="text-[11px] font-semibold uppercase tracking-wide text-muted">
                {t("crash.suggestions")}
              </div>
              <ul className="flex flex-col gap-[4px]">
                {rep.suggestions.map((s, i) => (
                  <li key={i} className="flex gap-[8px] text-[13px] leading-[1.6] text-fg">
                    <span className="select-none text-accent">·</span>
                    <span className="min-w-0">{s}</span>
                  </li>
                ))}
              </ul>
            </div>
          )}

          {/* 关键日志行(证据) */}
          {rep.matched && (
            <div className="flex flex-col gap-[4px]">
              <div className="text-[11px] font-semibold uppercase tracking-wide text-muted">
                {t("crash.evidence")}
              </div>
              <pre className="overflow-auto whitespace-pre-wrap break-words rounded-none border border-titlebar bg-panel-2 px-[10px] py-[8px] text-[12px] leading-[1.6] text-sub">
                {rep.matched}
              </pre>
            </div>
          )}

          {/* 可折叠的日志尾部 */}
          <div className="flex flex-col gap-[6px]">
            <button
              type="button"
              className="self-start text-[12px] font-semibold text-accent hover:underline"
              onClick={() => setShowLog((v) => !v)}
            >
              {showLog ? t("crash.hideLog") : t("crash.showLog")}
            </button>
            {showLog && (
              <pre className="max-h-[240px] overflow-auto whitespace-pre-wrap break-words rounded-none border border-titlebar bg-panel-2 px-[10px] py-[8px] text-[11px] leading-[1.6] text-sub">
                {rep.logTail || t("crash.noLog")}
              </pre>
            )}
          </div>
        </div>

        {/* 底部操作 */}
        <div className="flex items-center justify-between gap-[8px] border-t border-titlebar px-[20px] py-[14px]">
          <Button variant="ghost" onClick={openLogs}>
            {t("crash.openLogsDir")}
          </Button>
          <div className="flex items-center gap-[8px]">
            <Button variant="ghost" onClick={close}>
              {t("crash.close")}
            </Button>
            <Button variant="primary" onClick={copy}>
              {t("crash.copyDiagnostics")}
            </Button>
          </div>
        </div>
      </div>
    </Dialog>
  );
}

export default CrashDialog;
