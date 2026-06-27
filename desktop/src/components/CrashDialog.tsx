import { Component, For, Show, createSignal, createMemo } from "solid-js";
import { Dialog } from "./Dialog";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { crashReport, setCrashReport, type CrashReport } from "../store";
import { api } from "../ipc/api";
import { t } from "../i18n";

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
export const CrashDialog: Component = () => {
  const [showLog, setShowLog] = createSignal(false);
  const r = createMemo(() => crashReport());

  const close = () => {
    setCrashReport(null);
    setShowLog(false);
  };

  const copy = async () => {
    const rep = r();
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

  return (
    <Show when={r()}>
      {(rep) => (
        <Dialog
          open
          onClose={close}
          label={t("crash.title")}
          contentClass="w-[560px] max-w-[calc(100vw-48px)]"
        >
          <div class="flex max-h-[80vh] flex-col">
            {/* 头部:标题 + 副标题 */}
            <div class="flex flex-col gap-[6px] border-b border-titlebar px-[20px] pb-[16px] pt-[18px]">
              <div class="flex items-center gap-[8px]">
                <Heading size="sub">{t("crash.title")}</Heading>
                <span class="rounded-none border border-danger/40 bg-danger-soft px-[8px] py-[2px] text-[11px] font-semibold text-danger-text">
                  {categoryLabel(rep().category)}
                </span>
              </div>
              <div class="text-[12px] leading-[1.7] text-sub">{t("crash.subtitle")}</div>
            </div>

            {/* 主体:可滚动 */}
            <div class="flex min-h-0 flex-1 flex-col gap-[14px] overflow-auto px-[20px] py-[16px]">
              {/* 元信息网格 */}
              <div class="grid grid-cols-[auto_1fr] gap-x-[16px] gap-y-[6px] text-[13px]">
                <span class="text-muted">{t("crash.instance")}</span>
                <span class="min-w-0 truncate text-fg">{rep().name}</span>
                <Show when={versionLine(rep())}>
                  <span class="text-muted">{t("crash.version")}</span>
                  <span class="min-w-0 truncate text-fg">{versionLine(rep())}</span>
                </Show>
                <span class="text-muted">{t("crash.exitCode")}</span>
                <span class="text-fg">{rep().code ?? "—"}</span>
              </div>

              {/* 原因 */}
              <Show when={rep().reason}>
                <div class="flex flex-col gap-[4px]">
                  <div class="text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {t("crash.reason")}
                  </div>
                  <div class="text-[13px] leading-[1.7] text-fg">{rep().reason}</div>
                </div>
              </Show>

              {/* 建议 */}
              <Show when={rep().suggestions.length > 0}>
                <div class="flex flex-col gap-[6px]">
                  <div class="text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {t("crash.suggestions")}
                  </div>
                  <ul class="flex flex-col gap-[4px]">
                    <For each={rep().suggestions}>
                      {(s) => (
                        <li class="flex gap-[8px] text-[13px] leading-[1.6] text-fg">
                          <span class="select-none text-accent">·</span>
                          <span class="min-w-0">{s}</span>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
              </Show>

              {/* 关键日志行(证据) */}
              <Show when={rep().matched}>
                <div class="flex flex-col gap-[4px]">
                  <div class="text-[11px] font-semibold uppercase tracking-wide text-muted">
                    {t("crash.evidence")}
                  </div>
                  <pre class="overflow-auto whitespace-pre-wrap break-words rounded-none border border-titlebar bg-panel-2 px-[10px] py-[8px] text-[12px] leading-[1.6] text-sub">
                    {rep().matched}
                  </pre>
                </div>
              </Show>

              {/* 可折叠的日志尾部 */}
              <div class="flex flex-col gap-[6px]">
                <button
                  type="button"
                  class="self-start text-[12px] font-semibold text-accent hover:underline"
                  onClick={() => setShowLog((v) => !v)}
                >
                  {showLog() ? t("crash.hideLog") : t("crash.showLog")}
                </button>
                <Show when={showLog()}>
                  <pre class="max-h-[240px] overflow-auto whitespace-pre-wrap break-words rounded-none border border-titlebar bg-panel-2 px-[10px] py-[8px] text-[11px] leading-[1.6] text-sub">
                    {rep().logTail || t("crash.noLog")}
                  </pre>
                </Show>
              </div>
            </div>

            {/* 底部操作 */}
            <div class="flex items-center justify-between gap-[8px] border-t border-titlebar px-[20px] py-[14px]">
              <Button variant="ghost" onClick={openLogs}>
                {t("crash.openLogsDir")}
              </Button>
              <div class="flex items-center gap-[8px]">
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
      )}
    </Show>
  );
};

export default CrashDialog;
