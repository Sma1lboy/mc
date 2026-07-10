import { useEffect, useState } from "react";
import { fmtSize, fmtDate } from "../util/format";
import clsx from "clsx";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner } from "../components/Spinner";
import { Select } from "../components/Select";
import { Panel } from "../components/Panel";
import { Tag } from "../components/Tag";
import { Heading } from "../components/Typography";
import { LOADER_BADGE_TINT, ACCENT_BTN, ACCENT_BTN_COMPACT } from "../components/styles";
import { toast } from "../components/Toast";
import type { ModpackHit } from "../components/ModpackCard";
import { api } from "../ipc/api";
import { cached } from "../ipc/cache";
import { useAsync } from "../util/useAsync";
import { useAppStore, activeRoot } from "../store";
import { t, useLang } from "../i18n";
import type { InstanceSummary, ModrinthProject, ModrinthVersion, PackKind, ProjectKind } from "../ipc/types";
import { renderMarkdown } from "../util/markdown";
import { acceptedLoaders } from "../util/loaders";
import "./ModpackDetail.css";

type InstallableKind = Exclude<ProjectKind, "modpack">;

const KIND_META = (): Record<
  InstallableKind,
  { title: string; target: "mod" | "resourcepack" | "shader" | "datapack"; packKind: PackKind | null }
> => ({
  mod: { title: t("discover.kindMetaMod"), target: "mod", packKind: null },
  resourcepack: { title: t("discover.kindMetaResourcepack"), target: "resourcepack", packKind: "resource_pack" },
  shader: { title: t("discover.kindMetaShader"), target: "shader", packKind: "shader" },
  datapack: { title: t("discover.kindMetaDatapack"), target: "datapack", packKind: "datapack" },
});

const loaderLabel = (loader: string) =>
  ({ vanilla: t("discover.loaderVanilla"), forge: "Forge", neoforge: "NeoForge", fabric: "Fabric", quilt: "Quilt" } as Record<string, string>)[
    loader
  ] ?? loader;

const typeLabel = (type: string) =>
  ({ release: t("discover.typeRelease"), beta: t("discover.typeBeta"), alpha: t("discover.typeAlpha") } as Record<string, string>)[type] ?? type;

function versionMatches(version: ModrinthVersion, inst: InstanceSummary, kind: InstallableKind): boolean {
  if (!version.game_versions.includes(inst.mc_version)) return false;
  if (kind !== "mod") return true;
  if (inst.loader === "vanilla") return false;
  // Quilt 实例也接受 fabric 版本。
  return acceptedLoaders(inst.loader).some((l) => version.loaders.includes(l));
}

function compatibleVersionsFor(
  versions: ModrinthVersion[],
  inst: InstanceSummary,
  kind: InstallableKind,
): ModrinthVersion[] {
  return versions.filter((version) => versionMatches(version, inst, kind));
}

interface ProjectInstallDetailProps {
  hit: ModpackHit;
  kind: InstallableKind;
  onBack: () => void;
  /** 锁定安装目标:从实例详情进入时传入,隐藏「安装到实例」选择器,只装到该实例。 */
  lockedInstance?: InstanceSummary;
  /** 安装成功回调:实例模式下用来刷新「已安装」列表。 */
  onInstalled?: () => void;
  /** 内容来源平台(modrinth / curseforge);决定走哪个 provider 取版本/安装。缺省 modrinth。 */
  provider?: "modrinth" | "curseforge";
}

export default function ProjectInstallDetail(props: ProjectInstallDetailProps) {
  useLang();
  const instanceList = useAppStore((s) => s.instances);
  const meta = () => KIND_META()[props.kind];
  const lockMode = () => !!props.lockedInstance;
  const provider = () => props.provider ?? "modrinth";
  // CurseForge 作者禁第三方分发时,后端把文件计入 report.blocked;此时不算成功,提示去网页手动下载。
  const warnIfBlocked = (n: number) => {
    if (n > 0) toast({ type: "warn", message: t("discover.blockedManual", { count: n }) });
    return n > 0;
  };

  // Esc 返回上一层(与列表/灯箱一致的导航直觉)。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        props.onBack();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [props]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [installingVersion, setInstallingVersion] = useState<string | null>(null);
  const [openAbout, setOpenAbout] = useState(true);

  // 实例列表来自全局 store(单一真相);锁定模式直接用 props.lockedInstance,不读它。
  const { data: projectData, loading: projectLoading } = useAsync(
    () =>
      cached(`project|${provider()}|${props.hit.id}`, () => api.modrinthProject(props.hit.id, provider())).catch((e) => {
        toast({ type: "error", message: t("discover.aboutLoadFailed", { error: String(e) }) });
        return null as ModrinthProject | null;
      }),
    [props.hit.id],
  );
  const project = () => projectData;
  const { data: versionsData, loading: versionsLoading } = useAsync(
    () =>
      cached(`versions|${provider()}|${props.hit.id}`, () => api.modrinthVersions(props.hit.id, provider())).catch((e) => {
        toast({ type: "error", message: t("discover.versionsLoadFailed", { error: String(e) }) });
        return [] as ModrinthVersion[];
      }),
    [props.hit.id],
  );

  // 数据包逐存档生效:选好实例后还要选目标存档(saves/<world>/datapacks)。
  const isDatapack = () => props.kind === "datapack";
  const { data: worldsData } = useAsync(
    () => (isDatapack() && selectedId ? api.instanceWorlds(activeRoot(), selectedId) : Promise.resolve([])),
    [props.kind, selectedId],
  );
  const worlds = () => worldsData;
  const [world, setWorld] = useState<string | null>(null);
  useEffect(() => {
    if (!isDatapack()) return;
    const w = worlds() ?? [];
    if (w.length === 0) setWorld(null);
    else if (!world || !w.some((x) => x.folder === world)) setWorld(w[0].folder);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [props.kind, worldsData, world]);
  const worldArg = () => (isDatapack() ? world : null);

  const list = () => (props.lockedInstance ? [props.lockedInstance] : instanceList ?? []);
  const versionList = () => versionsData ?? [];
  const selectedInstance = () => list().find((inst) => inst.id === selectedId) ?? null;
  const compatibleFor = (inst: InstanceSummary) => compatibleVersionsFor(versionList(), inst, props.kind);
  const canInstallTo = (inst: InstanceSummary) => {
    if (props.kind !== "mod") return true;
    if (inst.loader === "vanilla") return false;
    return versionsLoading || versionList().length === 0 || compatibleFor(inst).length > 0;
  };
  const installableInstances = () => list().filter(canInstallTo);

  useEffect(() => {
    const current = selectedInstance();
    if (current && canInstallTo(current)) return;
    const preferred = installableInstances()[0] ?? list()[0];
    setSelectedId(preferred?.id ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instanceList, props.lockedInstance, versionsData, versionsLoading, selectedId, props.kind]);

  const links = () => {
    const p = project();
    if (!p) return [] as { label: string; url: string }[];
    return (
      [
        { label: t("discover.linkSource"), url: p.source_url },
        { label: t("discover.linkIssues"), url: p.issues_url },
        { label: t("discover.linkWiki"), url: p.wiki_url },
        { label: t("discover.linkDiscord"), url: p.discord_url },
      ].filter((link) => !!link.url) as { label: string; url: string }[]
    );
  };

  // 显式选版安装(走 install_version_file,不解析依赖)到当前选中的实例。
  async function installVersion(version: ModrinthVersion) {
    const inst = selectedInstance();
    if (!inst || installing || installingVersion) return;
    if (props.kind === "mod" && !versionMatches(version, inst, props.kind)) {
      toast({ type: "error", message: t("discover.incompatibleVersion") });
      return;
    }
    if (isDatapack() && !world) {
      toast({ type: "warn", message: t("discover.datapackNeedWorld") });
      return;
    }
    setInstallingVersion(version.id);
    try {
      // mod 传 mc/loader 以便一并解析 required 依赖;packs 不需要。
      const isMod = props.kind === "mod";
      const report = await api.installVersionFile(
        activeRoot(),
        inst.id,
        meta().target,
        version.id,
        isMod ? inst.mc_version : null,
        isMod ? inst.loader : null,
        worldArg(),
        provider(),
        props.hit.id,
      );
      if (warnIfBlocked(report.blocked?.length ?? 0)) return;
      const parts = [t("discover.installedVersionTo", { version: version.version_number, instance: inst.name || inst.id })];
      if (report.installed_deps > 0) parts.push(t("discover.depsAdded", { count: report.installed_deps }));
      if (report.unresolved.length > 0) parts.push(t("discover.depsUnresolved", { count: report.unresolved.length }));
      const conflicts = report.incompatible?.length ?? 0;
      if (conflicts > 0) parts.push(t("discover.declaredConflicts", { count: conflicts }));
      toast({
        type: report.unresolved.length > 0 || conflicts > 0 ? "warn" : "success",
        message: parts.join(","),
      });
      props.onInstalled?.();
    } catch (e) {
      toast({ type: "error", message: t("discover.installFailed", { error: String(e) }) });
    } finally {
      setInstallingVersion(null);
    }
  }

  async function installLatest() {
    const inst = selectedInstance();
    if (!inst || installing || installingVersion) return;
    if (!canInstallTo(inst)) {
      toast({ type: "error", message: t("discover.noCompatibleInstance") });
      return;
    }
    if (isDatapack() && !world) {
      toast({ type: "warn", message: t("discover.datapackNeedWorld") });
      return;
    }
    setInstalling(true);
    try {
      if (props.kind === "mod") {
        const report = await api.installMod(activeRoot(), inst.id, props.hit.id, inst.mc_version, inst.loader, provider());
        if (warnIfBlocked(report.blocked?.length ?? 0)) return;
        const parts = [t("discover.installedFiles", { count: report.installed.length })];
        if (report.unresolved.length > 0) parts.push(t("discover.depsUnresolved", { count: report.unresolved.length }));
        const conflicts = report.incompatible?.length ?? 0;
        if (conflicts > 0) parts.push(t("discover.declaredConflicts", { count: conflicts }));
        toast({
          type: report.unresolved.length > 0 || conflicts > 0 ? "warn" : "success",
          message: `${props.hit.title}:${parts.join(",")}`,
        });
      } else {
        const report = await api.installPack(activeRoot(), inst.id, meta().packKind!, props.hit.id, inst.mc_version, worldArg(), provider());
        if (warnIfBlocked(report.blocked?.length ?? 0)) return;
        toast({ type: "success", message: t("discover.installedToFile", { instance: inst.name || inst.id, file: report.file }) });
      }
      props.onInstalled?.();
    } catch (e) {
      toast({ type: "error", message: t("discover.installFailed", { error: String(e) }) });
    } finally {
      setInstalling(false);
    }
  }

  return (
    <div className="flex flex-col gap-[16px] px-[2px] pt-[4px] pb-[24px] overflow-y-auto">
      <button
        className="self-start bg-transparent border-none text-accent text-[14px] cursor-pointer py-[4px] px-0 transition-opacity duration-[var(--dur)] ease-app hover:opacity-70"
        onClick={props.onBack}
      >
        {t("discover.back")}
      </button>

      <div className="grid grid-cols-[minmax(0,1fr)_320px] gap-[18px] max-[980px]:grid-cols-1">
        <div className="min-w-0 flex flex-col gap-[16px]">
          <section className="flex flex-col gap-[12px]">
            {props.hit.gallery_url && (
              <img
                className="w-full max-h-[220px] object-cover rounded-none shadow-sunken"
                src={props.hit.gallery_url}
                alt=""
                width="960"
                height="540"
              />
            )}
            <div className="flex items-center gap-[14px]">
              {props.hit.icon_url ? (
                <img
                  className="w-[70px] h-[70px] rounded-none object-cover flex-[0_0_auto] shadow-sunken"
                  src={props.hit.icon_url}
                  alt=""
                  width="70"
                  height="70"
                  style={{ imageRendering: "pixelated" }}
                />
              ) : (
                <Panel variant="raised" className="w-[70px] h-[70px] flex items-center justify-center font-display text-[30px] text-strong bg-panel-2">
                  {(props.hit.title[0] ?? "?").toUpperCase()}
                </Panel>
              )}
              <div className="min-w-0">
                <div className="text-[12px] font-semibold text-tag">{meta().title}</div>
                <Heading as="h1" size="page" className="whitespace-nowrap overflow-hidden text-ellipsis">{props.hit.title}</Heading>
                <div className="mt-[4px] text-[13px] text-sub">
                  by {props.hit.author} · ⬇ {props.hit.downloads.toLocaleString()}
                  {!!project()?.followers && " · ♥ " + project()!.followers.toLocaleString()}
                </div>
              </div>
            </div>
            <p className="m-0 text-[14px] leading-[1.7] text-sub">{props.hit.description}</p>
            <div className="flex flex-wrap gap-[6px]">
              {props.hit.categories.map((category) => (
                <Tag key={category}>{category}</Tag>
              ))}
            </div>
          </section>

          <section className="flex flex-col gap-[10px]">
            <Panel
              as="button"
              variant="raised"
              className="flex items-center justify-between border-none bg-panel-3 px-[14px] py-[11px] text-left cursor-pointer active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
              onClick={() => setOpenAbout((v) => !v)}
            >
              <Heading size="sub">{t("discover.projectAbout")}</Heading>
              <span className="text-muted">{openAbout ? "⌃" : "⌄"}</span>
            </Panel>
            {openAbout && (
              <Panel variant="sunken" className="bg-panel px-[14px] py-[12px]">
                {projectLoading ? (
                  <div className="flex items-center gap-[10px] text-muted text-[13px]">
                    <Spinner size={16} /> {t("discover.loadingAbout")}
                  </div>
                ) : project()?.body?.trim() ? (
                  <div className="md text-[14px] leading-[1.75] text-sub" dangerouslySetInnerHTML={{ __html: renderMarkdown(project()!.body) }} />
                ) : (
                  <div className="text-muted text-[13px]">{t("discover.noAboutBody")}</div>
                )}
              </Panel>
            )}
          </section>

          <section className="flex flex-col gap-[8px]">
            <div className="flex items-center justify-between">
              <Heading size="sub">{t("discover.tabVersions")}</Heading>
              <span className="text-[12px] text-muted">{t("discover.versionsCount", { count: versionList().length })}</span>
            </div>
            {versionsLoading ? (
              <div className="flex items-center gap-[10px] text-muted text-[13px] py-[8px]">
                <Spinner size={16} /> {t("discover.loadingVersions")}
              </div>
            ) : versionList().length === 0 ? (
              <div className="text-muted text-[13px] py-[10px]">{t("discover.noVersionsDot")}</div>
            ) : (
              <div className="flex flex-col gap-[6px] max-h-[360px] overflow-y-auto">
                {versionList().map((version) => {
                  const inst = selectedInstance();
                  const compatible = inst ? versionMatches(version, inst, props.kind) : false;
                  const busy = installing || installingVersion !== null;
                  // 仅 mod 的加载器/版本不匹配是硬性不可装(必崩);资源包/光影的版本差异
                  // 只是软提示(游戏内可带警告加载),不拦。
                  const blocked = props.kind === "mod" && !!inst && !compatible;
                  return (
                    <Panel
                      key={version.id}
                      variant="sunken"
                      className={clsx("flex items-center gap-[10px] bg-panel-2 px-[10px] py-[8px]", { "opacity-55": !!inst && !compatible })}
                    >
                      <div className="min-w-0 flex-1">
                        <div className="text-[13px] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                          {version.version_number}
                          <span className="ml-[6px] text-[11px] font-medium text-muted">{typeLabel(version.version_type)}</span>
                        </div>
                        <div className="mt-[2px] text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                          {version.game_versions.slice(0, 5).join(", ")}
                          {version.loaders.length > 0 && " · " + version.loaders.join(" / ")}
                          {" · "}
                          {fmtDate(version.date_published)}
                          {!!version.file_size && " · " + fmtSize(version.file_size)}
                        </div>
                      </div>
                      <span className="text-[11px] text-muted whitespace-nowrap">⬇ {version.downloads.toLocaleString()}</span>
                      <button
                        className={ACCENT_BTN_COMPACT}
                        disabled={busy || !inst || blocked}
                        title={!inst ? t("discover.selectInstanceFirst") : blocked ? t("discover.blockedTooltip") : ""}
                        onClick={() => installVersion(version)}
                      >
                        {installingVersion === version.id ? t("discover.installing") : t("discover.install")}
                      </button>
                    </Panel>
                  );
                })}
              </div>
            )}
          </section>
        </div>

        <aside className="flex flex-col gap-[12px]">
          {/* 全局模式才显示实例选择器;实例详情进入时目标已锁定。 */}
          {!lockMode() && (
            <Panel as="section" variant="sunken" className="bg-panel px-[14px] py-[14px]">
              <Heading size="sub" className="mb-[10px]">{t("discover.installToInstance")}</Heading>
              {instanceList === undefined ? (
                <div className="flex items-center gap-[10px] text-muted text-[13px]">
                  <Spinner size={16} /> {t("discover.loadingInstances")}
                </div>
              ) : list().length === 0 ? (
                <div className="text-[13px] leading-[1.7] text-muted">{t("discover.noInstances")}</div>
              ) : (
                <div className="flex flex-col gap-[6px] max-h-[310px] overflow-y-auto">
                  {list().map((inst) => {
                    const disabled = !canInstallTo(inst);
                    const compatCount = compatibleFor(inst).length;
                    const selected = selectedId === inst.id;
                    return (
                      <button
                        key={inst.id}
                        className={clsx(
                          "flex w-full items-center gap-[9px] rounded-none bg-panel-2 px-[9px] py-[8px] text-left cursor-pointer shadow-sunken transition-[box-shadow] duration-[var(--dur)] ease-app disabled:cursor-not-allowed disabled:opacity-50",
                          { "!bg-accent !shadow-raised": selected },
                        )}
                        disabled={disabled}
                        onClick={() => setSelectedId(inst.id)}
                      >
                        <span
                          className={`w-[30px] h-[30px] flex-[0_0_30px] rounded-none grid place-items-center bg-accent text-accent-text text-[13px] font-bold shadow-raised ${LOADER_BADGE_TINT}`}
                          data-loader={inst.loader}
                        >
                          {(inst.name || inst.id)[0]?.toUpperCase()}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span
                            className={clsx(
                              "block text-[13px] font-semibold whitespace-nowrap overflow-hidden text-ellipsis",
                              selected ? "text-accent-text" : "text-fg",
                            )}
                          >
                            {inst.name || inst.id}
                          </span>
                          <span
                            className={clsx(
                              "block text-[11px] whitespace-nowrap overflow-hidden text-ellipsis",
                              selected ? "text-accent-text" : "text-muted",
                            )}
                          >
                            {inst.mc_version} · {loaderLabel(inst.loader)}
                            {props.kind === "mod" && inst.loader !== "vanilla" && " · " + t("discover.matchingVersions", { count: compatCount })}
                          </span>
                        </span>
                      </button>
                    );
                  })}
                </div>
              )}
            </Panel>
          )}

          <Panel as="section" variant="sunken" className="bg-panel px-[14px] py-[14px]">
            {(() => {
              const inst = selectedInstance();
              if (!inst) return <div className="text-[13px] text-muted">{t("discover.selectInstanceToInstall")}</div>;
              return (
                <div className="flex flex-col gap-[10px]">
                  <div>
                    <div className="text-[12px] text-muted">{t("discover.target")}</div>
                    <Heading size="sub" className="mt-[2px]">{inst.name || inst.id}</Heading>
                    <div className="mt-[2px] text-[12px] text-muted">
                      Minecraft {inst.mc_version} · {loaderLabel(inst.loader)}
                    </div>
                  </div>

                  {/* 数据包目标存档:数据包逐存档生效,必须先选一个存档。 */}
                  {isDatapack() &&
                    ((worlds() ?? []).length > 0 ? (
                      <label className="flex items-center gap-[8px] text-[12px] text-muted">
                        <span className="shrink-0">{t("discover.targetWorld")}</span>
                        <Select
                          className="flex-1 !min-w-0"
                          value={world ?? ""}
                          onChange={(v) => setWorld(v)}
                          options={(worlds() ?? []).map((w) => ({ value: w.folder, label: w.name || w.folder }))}
                        />
                      </label>
                    ) : (
                      <div className="text-[12px] leading-[1.7] text-muted">
                        {t("discover.datapackNoWorlds")}
                      </div>
                    ))}

                  <button
                    className={ACCENT_BTN}
                    disabled={installing || installingVersion !== null || !canInstallTo(inst) || (isDatapack() && !world)}
                    onClick={installLatest}
                  >
                    {installing ? t("discover.installing") : t("discover.installLatest", { kind: meta().title })}
                  </button>
                  {props.kind === "mod" && inst.loader === "vanilla" && (
                    <div className="text-[12px] leading-[1.6] text-muted">{t("discover.vanillaNoModLoader")}</div>
                  )}
                  {props.kind === "mod" && inst.loader !== "vanilla" && !versionsLoading && compatibleFor(inst).length === 0 && (
                    <div className="text-[12px] leading-[1.6] text-muted">{t("discover.noMatchingFile")}</div>
                  )}
                  {links().length > 0 && (
                    <div className="flex flex-wrap gap-[6px] pt-[4px]">
                      {links().map((link) => (
                        <button
                          key={link.url}
                          className="h-[28px] rounded-none bg-panel-3 px-[10px] text-[12px] text-tag cursor-pointer shadow-raised active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
                          onClick={() => shellOpen(link.url)}
                        >
                          {link.label} ↗
                        </button>
                      ))}
                    </div>
                  )}
                </div>
              );
            })()}
          </Panel>
        </aside>
      </div>
    </div>
  );
}
