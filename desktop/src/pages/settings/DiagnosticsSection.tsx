import { useRef, useState } from "react";
import { Panel, Heading, Button, toast } from "../../components";
import { api } from "../../ipc/api";
import { t } from "../../i18n";

const sectionClass = "p-[20px]";

/** 诊断块:打开日志目录 + 应用内查看日志尾部。状态全部本地。 */
export function DiagnosticsSection() {
  // 应用内日志查看(诊断):按需读取最新日志文件的末尾,展开时滚到底看最新。
  const [logOpen, setLogOpen] = useState(false);
  const [logText, setLogText] = useState("");
  const [logLoading, setLogLoading] = useState(false);
  const logBox = useRef<HTMLPreElement>(null);
  async function loadLog() {
    setLogLoading(true);
    try {
      setLogText(await api.readLogTail(800));
    } catch (e) {
      setLogText(t("settings.logReadFailed", { err: String(e) }));
    } finally {
      setLogLoading(false);
      queueMicrotask(() => {
        if (logBox.current) logBox.current.scrollTop = logBox.current.scrollHeight;
      });
    }
  }
  function toggleLog() {
    const next = !logOpen;
    setLogOpen(next);
    if (next) void loadLog();
  }

  return (
            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionDiagnostics")}
              </Heading>
              <div className="flex items-center justify-between text-fg text-[14px]">
                <div className="flex flex-col gap-[2px] min-w-0">
                  <span>{t("settings.logs")}</span>
                  <span className="text-[12px] text-muted">{t("settings.logsDesc")}</span>
                </div>
                <div className="flex items-center gap-[8px] shrink-0">
                  <Button variant="ghost" onClick={toggleLog}>
                    {logOpen ? t("settings.hideLog") : t("settings.viewLog")}
                  </Button>
                  <Button
                    variant="ghost"
                    onClick={async () => {
                      try {
                        const dir = await api.openLogsDir();
                        await api.revealPath(dir);
                      } catch (e) {
                        toast({ type: "error", message: t("settings.openLogsDirFailed", { err: String(e) }) });
                      }
                    }}
                  >
                    {t("settings.openLogsDir")}
                  </Button>
                </div>
              </div>

              {logOpen && (
                <div className="mt-[12px]">
                  <div className="flex justify-end mb-[6px]">
                    <button
                      className="text-[12px] text-muted hover:text-fg disabled:opacity-50 cursor-pointer bg-transparent border-none transition-colors duration-150"
                      onClick={() => void loadLog()}
                      disabled={logLoading}
                    >
                      {logLoading ? t("settings.refreshing") : t("settings.refresh")}
                    </button>
                  </div>
                  <pre
                    ref={logBox}
                    className="max-h-[340px] overflow-auto m-0 bg-sidebar shadow-input rounded-none p-[12px] text-[11px] leading-[1.55] text-muted font-mono whitespace-pre-wrap break-all"
                  >
                    {logText || t("settings.logEmpty")}
                  </pre>
                </div>
              )}
            </Panel>
  );
}
