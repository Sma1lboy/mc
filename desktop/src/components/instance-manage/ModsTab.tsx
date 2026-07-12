import clsx from "clsx";
import { ContentBrowser } from "../ContentBrowser";
import { ErrorState } from "../ErrorState";
import { ACCENT_BTN } from "../styles";
import { Toggle } from "../Toggle";
import ProjectInstallDetail from "../../pages/ProjectInstallDetail";
import { Spinner } from "../Spinner";
import { openInstanceSubdir } from "../../util/instanceActions";
import { activeRoot } from "../../store";
import { t } from "../../i18n";
import type { InstanceSummary } from "../../ipc/types";
import { AddLoaderPanel } from "./AddLoaderPanel";
import { LABEL, INSTALL_BTN, DEL_BTN, OPEN_BTN } from "./shared";
import type { ModsTabState } from "./useModsTab";

/** Mods 标签的视图(纯展示;状态与动作由 useModsTab 提供,行为不变)。 */
export function ModsTab(props: {
  instance: InstanceSummary | null;
  /** 已合并「当前就在 mods 标签」的浏览态。 */
  browsing: boolean;
  onExitBrowse: () => void;
  startBrowse: () => void;
  m: ModsTabState;
  onLoaderAdded: (newId: string) => void;
}) {
  const {
    searchLoader, modDetail, setModDetail, modDetailProvider, setModDetailProvider,
    installing, addedMods, setAddedMods, installHit, refetchMods,
    mods, modsLoading, modsError, updates, checking, updating,
    checkUpdates, applyUpdate, applyAllUpdates, toggleMod, setConfirmDelMod,
  } = props.m;
  return (
    <>
            {searchLoader === null ? (
              <AddLoaderPanel instance={props.instance!} onAdded={props.onLoaderAdded} />
            ) : props.browsing ? (
              // 浏览模式 = 复用探索页:搜索列表 →(点进)详情安装,装完回到已安装。
              modDetail ? (
                <ProjectInstallDetail
                  hit={modDetail}
                  kind="mod"
                  provider={modDetailProvider}
                  lockedInstance={props.instance!}
                  onBack={() => setModDetail(null)}
                  onInstalled={() => {
                    refetchMods();
                    if (modDetail) setAddedMods((s) => new Set(s).add(modDetail.id));
                  }}
                />
              ) : (
                <>
                  <button
                    className="self-start inline-flex items-center gap-[4px] h-[28px] px-[10px] rounded-none border-none bg-transparent text-muted text-[12px] cursor-pointer transition-colors duration-150 hover:bg-panel-3 hover:text-fg"
                    onClick={props.onExitBrowse}
                  >
                    {t("instance.backToInstalled")}
                  </button>
                  <ContentBrowser
                    kind="mod"
                    mcVersion={props.instance?.mc_version ?? ""}
                    loader={searchLoader}
                    onOpenDetail={(hit, provider) => {
                      setModDetail(hit);
                      setModDetailProvider(provider);
                    }}
                    onAdd={(hit, provider) => installHit(hit.id, hit.title, provider)}
                    addingIds={installing}
                    addedIds={addedMods}
                    autofocus
                    onEscape={props.onExitBrowse}
                    placeholder={t("instance.searchModrinthMod", {
                      version: props.instance?.mc_version ?? "",
                      loader: searchLoader ?? t("instance.noLoader"),
                    })}
                  />
                </>
              )
            ) : (
              <>
                {/* 默认:「已安装」标题行,右侧聚拢动作(打开目录 + 检查更新 + 紧凑「添加」)。 */}
                <div className="flex items-center justify-between">
                  <div className={LABEL}>{t("instance.installedTitle")}</div>
                  <div className="flex items-center gap-[6px]">
                    <button
                      className={OPEN_BTN}
                      onClick={() => openInstanceSubdir(activeRoot(), props.instance!.id, "mods")}
                    >
                      {t("instance.openDir")}
                    </button>
                    <button
                      className="text-[12px] text-accent px-[8px] py-[3px] rounded-none cursor-pointer hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
                      disabled={checking || searchLoader === null}
                      onClick={checkUpdates}
                    >
                      {checking ? t("instance.checking") : t("instance.checkUpdates")}
                    </button>
                    <button
                      className="shrink-0 h-[28px] px-[10px] rounded-none bg-accent text-white shadow-raised text-[12px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed"
                      onClick={props.startBrowse}
                    >
                      {t("instance.add")}
                    </button>
                  </div>
                </div>

                {/* 可更新清单(检查后才出现) */}
                {(updates ?? []).length > 0 && (
                  <div className="flex flex-col gap-[6px] rounded-none bg-panel-2 p-[8px]">
                    <div className="flex items-center justify-between">
                      <span className="text-[12px] text-fg font-semibold">
                        {t("instance.updatesAvailable", { n: updates!.length })}
                      </span>
                      <button className={INSTALL_BTN} disabled={updating.size > 0} onClick={applyAllUpdates}>
                        {t("instance.updateAll")}
                      </button>
                    </div>
                    {updates!.map((u) => (
                      <div key={u.file_name} className="bg-panel-2 shadow-sunken flex items-center gap-[10px] py-[6px] px-[8px] rounded-none">
                        <div className="flex-1 min-w-0">
                          <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">{u.name}</div>
                          <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                            {(u.current_version ?? t("instance.currentVersion")) + " → " + u.new_version}
                          </div>
                        </div>
                        <button
                          className={INSTALL_BTN}
                          disabled={updating.has(u.file_name)}
                          onClick={() => applyUpdate(u)}
                        >
                          {updating.has(u.file_name) ? t("instance.updating") : t("instance.update")}
                        </button>
                      </div>
                    ))}
                  </div>
                )}

                {/* 已安装 mod 列表 */}
                {modsLoading ? (
                  <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
                    <Spinner size={16} /> {t("instance.scanningMods")}
                  </div>
                ) : (mods ?? []).length > 0 ? (
                  <div className="flex flex-col gap-[6px]">
                    {mods!.map((m) => (
                      <div
                        key={m.file_name}
                        className={clsx("flex items-center gap-[10px] py-[8px] px-[10px] rounded-none bg-panel-2", {
                          "opacity-55": !m.enabled,
                        })}
                      >
                        <div className="flex-1 min-w-0">
                          <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">{m.name}</div>
                          <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                            {[m.version, m.loader, m.file_name].filter(Boolean).join(" · ")}
                          </div>
                        </div>
                        <div className="flex items-center shrink-0">
                          <Toggle checked={m.enabled} onChange={(v) => toggleMod(m, v)} title={t("instance.enable")} />
                        </div>
                        <button className={DEL_BTN} onClick={() => setConfirmDelMod(m)}>
                          {t("instance.delete")}
                        </button>
                      </div>
                    ))}
                  </div>
                ) : modsError ? (
                  <ErrorState compact message={t("instance.modListError")} onRetry={() => void refetchMods()} />
                ) : (
                  <div className="flex flex-col items-center justify-center gap-[12px] py-[40px] text-center">
                    <div className="text-muted text-[13px]">{t("instance.noMods")}</div>
                    <button className={ACCENT_BTN} onClick={props.startBrowse}>
                      {t("instance.addMod")}
                    </button>
                  </div>
                )}
              </>
            )}
    </>
  );
}
