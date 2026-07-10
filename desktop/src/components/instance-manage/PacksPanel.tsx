import { useEffect, useState } from "react";
import clsx from "clsx";
import { ContentBrowser, type ContentProvider } from "../ContentBrowser";
import { ErrorState } from "../ErrorState";
import { ACCENT_BTN } from "../styles";
import { Toggle } from "../Toggle";
import { Dialog } from "../Dialog";
import { Button } from "../Button";
import { Select } from "../Select";
import ProjectInstallDetail from "../../pages/ProjectInstallDetail";
import type { ModpackHit } from "../ModpackCard";
import { Spinner } from "../Spinner";
import { toast } from "../Toast";
import { api } from "../../ipc/api";
import { openInstanceSubdir } from "../../util/instanceActions";
import { activeRoot } from "../../store";
import { useAsync } from "../../util/useAsync";
import { t } from "../../i18n";
import type { InstanceSummary, PackKind, PackInfo, ProjectKind, WorldInfo } from "../../ipc/types";
import { LABEL, DEL_BTN, OPEN_BTN, fmtSize } from "./shared";

/**
 * PacksPanel —— 资源包 / 光影的统一面板:Modrinth 搜索安装 + 本地启停/删除。
 * 与 Mods 面板同构,差异仅在 PackKind / 搜索资源类型;按本实例 mc 版本过滤,
 * 资源包/光影在 Modrinth 上不按加载器细分,故 loader 传 null。
 */
export function PacksPanel(props: {
  instance: InstanceSummary;
  kind: PackKind;
  searchKind: ProjectKind;
  emptyHint: string;
  /** 外部导入计数:递增即触发重扫(拖拽导入后由父组件 bump)。 */
  tick?: number;
  /** 受控的「浏览/添加」模式(由父组件统一持有,用于隐藏详情页头部)。 */
  browse: boolean;
  onBrowse: (v: boolean) => void;
}) {
  // 数据包逐存档生效:落到 saves/<world>/datapacks。其它包类型无 world 概念。
  const isDatapack = props.kind === "datapack";
  const { data: worlds } = useAsync<WorldInfo[] | undefined>(
    () => (isDatapack ? api.instanceWorlds(activeRoot(), props.instance.id) : Promise.resolve(undefined)),
    [isDatapack, props.instance.id],
  );
  const [world, setWorld] = useState<string | null>(null);
  // 默认选中第一个存档(按上次游玩排序);存档变化后若当前选中已失效则回退。
  useEffect(() => {
    if (!isDatapack) return;
    const list = worlds ?? [];
    setWorld((prev) => {
      if (list.length === 0) return null;
      if (!prev || !list.some((w) => w.folder === prev)) return list[0].folder;
      return prev;
    });
  }, [isDatapack, worlds]);
  const worldArg = isDatapack ? world : null;

  const { data: packs, loading: packsLoading, error: packsError, refetch } = useAsync<PackInfo[]>(
    () => api.instancePacks(activeRoot(), props.instance.id, props.kind, worldArg),
    [props.instance.id, props.kind, props.tick ?? 0, worldArg],
  );

  const [detail, setDetail] = useState<ModpackHit | null>(null);
  // 详情页对应的来源平台(随 onOpenDetail 一起带过来),决定详情里取版本/安装走哪个 provider。
  const [detailProvider, setDetailProvider] = useState<ContentProvider>("modrinth");
  // 后台并行安装:正在安装的 project_id 集合(不阻塞其它行)。
  const [installing, setInstalling] = useState<Set<string>>(new Set());
  // 本次浏览已添加的 project_id:行按钮即时变「已添加」。
  const [added, setAdded] = useState<Set<string>>(new Set());
  // 删除资源包前确认(删除是破坏性的,与存档删除一致)。
  const [confirmDel, setConfirmDel] = useState<PackInfo | null>(null);
  const startBrowse = () => {
    setAdded(new Set<string>());
    props.onBrowse(true);
  };

  // 行内「下载」:直接装最新兼容版(资源包/光影/数据包不分加载器),后台并行不阻塞其它行。
  async function install(projectId: string, title: string, provider: ContentProvider = "modrinth") {
    if (installing.has(projectId)) return;
    setInstalling((s) => new Set(s).add(projectId));
    try {
      const report = await api.installPack(
        activeRoot(),
        props.instance.id,
        props.kind,
        projectId,
        props.instance.mc_version,
        worldArg,
        provider,
      );
      if ((report.blocked?.length ?? 0) > 0) {
        toast({ type: "warn", message: t("instance.blockedManual", { count: report.blocked!.length }) });
      } else {
        toast({ type: "success", message: t("instance.installed", { title, file: report.file }) });
        setAdded((s) => new Set(s).add(projectId));
        refetch();
      }
    } catch (e) {
      toast({ type: "error", message: t("instance.installFailed", { err: String(e) }) });
    } finally {
      setInstalling((s) => {
        const n = new Set(s);
        n.delete(projectId);
        return n;
      });
    }
  }

  async function toggle(p: PackInfo, enabled: boolean) {
    try {
      await api.setPackEnabled(activeRoot(), props.instance.id, props.kind, p.file_name, enabled, worldArg);
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.opFailed", { err: String(e) }) });
    }
  }

  async function remove(p: PackInfo) {
    try {
      await api.deletePack(activeRoot(), props.instance.id, props.kind, p.file_name, worldArg);
      toast({ type: "success", message: t("instance.deletedFile", { file: p.file_name }) });
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(e) }) });
    }
  }

  return (
    <div className="flex flex-col gap-[8px]">
      {/* 数据包目标存档选择器:数据包是逐存档生效的,必须先选一个存档。 */}
      {isDatapack &&
        ((worlds ?? []).length > 0 ? (
          <label className="flex items-center gap-[8px] text-[12px] text-muted">
            <span className="shrink-0">{t("instance.targetWorld")}</span>
            <Select
              className="flex-1 !min-w-0"
              value={world ?? ""}
              onChange={(v) => setWorld(v)}
              options={(worlds ?? []).map((w) => ({ value: w.folder, label: w.name || w.folder }))}
            />
          </label>
        ) : (
          <div className="text-[12px] leading-[1.7] text-muted py-[4px]">{t("instance.datapackNoWorlds")}</div>
        ))}

      {props.browse ? (
        // 浏览模式 = 复用探索页:搜索列表 →(点进)详情安装,装完回到已安装。
        detail ? (
          <ProjectInstallDetail
            hit={detail}
            kind={props.searchKind as Exclude<ProjectKind, "modpack">}
            provider={detailProvider}
            lockedInstance={props.instance}
            onBack={() => setDetail(null)}
            onInstalled={() => {
              refetch();
              if (detail) setAdded((s) => new Set(s).add(detail.id));
            }}
          />
        ) : (
          <>
            <button
              className="self-start inline-flex items-center gap-[4px] h-[28px] px-[10px] rounded-none border-none bg-transparent text-muted text-[12px] cursor-pointer transition-colors duration-150 hover:bg-panel-3 hover:text-fg"
              onClick={() => {
                setDetail(null);
                props.onBrowse(false);
              }}
            >
              {t("instance.backToInstalled")}
            </button>
            <ContentBrowser
              kind={props.searchKind}
              mcVersion={props.instance.mc_version}
              loader={null}
              onOpenDetail={(hit, provider) => {
                setDetail(hit);
                setDetailProvider(provider);
              }}
              onAdd={(hit, provider) => install(hit.id, hit.title, provider)}
              addingIds={installing}
              addedIds={added}
              disabledReason={
                isDatapack ? () => (worldArg ? null : t("instance.selectTargetWorldFirst")) : undefined
              }
              autofocus
              onEscape={() => props.onBrowse(false)}
              placeholder={t("instance.searchModrinth", { version: props.instance.mc_version })}
            />
          </>
        )
      ) : (
        <>
          {/* 默认:「已安装」标题行,右侧聚拢动作(打开目录 + 紧凑「添加」)。 */}
          <div className="flex items-center justify-between">
            <div className={LABEL}>{t("instance.installedTitle")}</div>
            <div className="flex items-center gap-[6px]">
              <button
                className={OPEN_BTN}
                onClick={() =>
                  openInstanceSubdir(
                    activeRoot(),
                    props.instance.id,
                    props.kind === "resource_pack"
                      ? "resourcepacks"
                      : props.kind === "shader"
                        ? "shaderpacks"
                        : world
                          ? `saves/${world}/datapacks`
                          : "datapacks",
                  )
                }
              >
                {t("instance.openDir")}
              </button>
              <button
                className="shrink-0 h-[28px] px-[10px] rounded-none bg-accent text-white shadow-raised text-[12px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed"
                onClick={startBrowse}
              >
                {t("instance.add")}
              </button>
            </div>
          </div>

          {packsLoading ? (
            <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
              <Spinner size={16} /> {t("instance.scanning")}
            </div>
          ) : (packs ?? []).length > 0 ? (
            <div className="flex flex-col gap-[6px]">
              {packs!.map((p) => (
                <div
                  key={p.file_name}
                  className={clsx("flex items-center gap-[10px] py-[8px] px-[10px] rounded-none bg-panel-2", {
                    "opacity-55": !p.enabled,
                  })}
                >
                  <div className="flex-1 min-w-0">
                    <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                      {p.file_name.replace(/\.disabled$/, "")}
                    </div>
                    <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                      {[p.description, fmtSize(p.size)].filter(Boolean).join(" · ")}
                    </div>
                  </div>
                  <div className="flex items-center shrink-0">
                    <Toggle checked={p.enabled} onChange={(v) => toggle(p, v)} title={t("instance.enable")} />
                  </div>
                  <button className={DEL_BTN} onClick={() => setConfirmDel(p)}>
                    {t("instance.delete")}
                  </button>
                </div>
              ))}
            </div>
          ) : packsError ? (
            <ErrorState compact message={t("instance.loadFailedShort")} onRetry={() => void refetch()} />
          ) : (
            <div className="flex flex-col items-center justify-center gap-[12px] py-[40px] text-center">
              <div className="text-muted text-[13px]">{props.emptyHint}</div>
              <button className={ACCENT_BTN} onClick={startBrowse}>
                {t("instance.add")}
              </button>
            </div>
          )}
        </>
      )}

      <Dialog
        open={confirmDel !== null}
        onClose={() => setConfirmDel(null)}
        label={t("instance.deleteResourcePack")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <div className="text-[15px] font-semibold text-fg break-words">
            {t("instance.deleteFileConfirm", { file: confirmDel?.file_name.replace(/\.disabled$/, "") ?? "" })}
          </div>
          <div className="text-[13px] text-muted leading-[1.6]">{t("instance.deleteFileBody")}</div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDel(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const p = confirmDel;
                setConfirmDel(null);
                if (p) void remove(p);
              }}
            >
              {t("instance.delete")}
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}
