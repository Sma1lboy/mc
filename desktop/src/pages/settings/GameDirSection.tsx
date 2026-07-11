import clsx from "clsx";
import { Button, Panel, Heading, Spinner, toast } from "../../components";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../ipc/api";
import { useAsync } from "../../util/useAsync";
import { useAppStore, setCurrentRoot } from "../../store";
import { t } from "../../i18n";
import type { GlobalSettings } from "../../ipc/types";

const sectionClass = "p-[20px]";

/** 游戏目录块:已发现的根 + 自定义根增删/切换。设置状态由父页持有。 */
export function GameDirSection({
  settings,
  setSettings,
}: {
  settings: GlobalSettings | null;
  setSettings: (s: GlobalSettings) => void;
}) {
  const currentRoot = useAppStore((s) => s.currentRoot);
  // 游戏目录(根)：列出已发现的根 + 让用户切换/增删自定义根。
  const roots = useAsync(() => api.listRoots(), []);

  // 把自定义根写盘后再让后端重新发现:setSettings 是异步落盘,必须 await 再 refetch,
  // 否则 list_roots 可能读到旧设置(看不到刚加的根)。
  async function persistCustomRoots(customRoots: string[]) {
    if (!settings) return;
    const next = { ...settings, custom_roots: customRoots };
    setSettings(next);
    try {
      await api.setSettings(next);
      roots.refetch();
    } catch (e) {
      toast({ type: "error", message: t("settings.saveDirFailed", { err: String(e) }) });
    }
  }

  async function addCustomRoot() {
    const picked = await openDialog({ directory: true, title: t("settings.pickGameDir") }).catch(() => null);
    if (!picked || typeof picked !== "string") return;
    // 已在已发现列表(便携/官方/已有自定义)里 → 直接切过去,不往设置里塞重复项。
    if ((roots.data ?? []).some((r) => r.path === picked)) {
      setCurrentRoot(picked);
      return;
    }
    const list = settings?.custom_roots ?? [];
    if (!list.includes(picked)) await persistCustomRoots([...list, picked]);
    setCurrentRoot(picked); // 切到新加的根
  }

  async function removeCustomRoot(path: string) {
    const list = (settings?.custom_roots ?? []).filter((p) => p !== path);
    await persistCustomRoots(list);
    if (currentRoot === path) {
      // 删的是当前根:落到剩余的第一个(没有则交回后端默认)。
      setCurrentRoot((roots.data ?? [])[0]?.path ?? null);
    }
  }

  return (
            <Panel variant="sunken" className={sectionClass}>
              <Heading size="sub" as="h2" className="mb-[14px]">
                {t("settings.sectionGameDir")}
              </Heading>
              {roots.loading ? (
                <div className="flex justify-center p-[20px]"><Spinner /></div>
              ) : (
                <div className="flex flex-col gap-[8px]">
                  {(roots.data ?? []).map((r) => {
                    const active = (currentRoot ?? "") === r.path;
                    return (
                      <Panel
                        key={r.path}
                        variant="raised"
                        className={clsx("flex items-center gap-[10px] py-[9px] px-[12px]", { "ring-2 ring-accent": active })}
                      >
                        <button
                          className="flex-1 min-w-0 flex flex-col gap-[2px] text-left bg-transparent border-none p-0 cursor-pointer"
                          onClick={() => setCurrentRoot(r.path)}
                          title={t("settings.setAsCurrent")}
                        >
                          <span className="flex items-center gap-[6px] text-[13px] text-fg">
                            {r.name}
                            {active && (
                              <span className="text-accent text-[12px]" aria-hidden="true">{t("settings.current")}</span>
                            )}
                          </span>
                          <span className="text-[11px] text-faint break-all">{r.path}</span>
                        </button>
                        {r.kind === "custom" && (
                          <button
                            className="shrink-0 text-[12px] text-danger-text px-[8px] py-[4px] rounded-none cursor-pointer hover:bg-danger-soft"
                            onClick={() => void removeCustomRoot(r.path)}
                          >
                            {t("settings.remove")}
                          </button>
                        )}
                      </Panel>
                    );
                  })}
                  <Button variant="primary" className="self-start" onClick={() => void addCustomRoot()}>
                    {t("settings.addDir")}
                  </Button>
                </div>
              )}
            </Panel>
  );
}
