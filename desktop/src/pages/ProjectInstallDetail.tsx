import { Component, createEffect, createResource, createSignal, onCleanup, onMount, For, Show } from "solid-js";
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
import { activeRoot, instances } from "../store";
import { t } from "../i18n";
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

function fmtDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleDateString();
}

function fmtSize(size: number | null): string {
  if (!size) return "";
  return size >= 1024 * 1024 ? `${(size / 1024 / 1024).toFixed(1)} MB` : `${Math.ceil(size / 1024)} KB`;
}

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

const ProjectInstallDetail: Component<{
  hit: ModpackHit;
  kind: InstallableKind;
  onBack: () => void;
  /** 锁定安装目标:从实例详情进入时传入,隐藏「安装到实例」选择器,只装到该实例。 */
  lockedInstance?: InstanceSummary;
  /** 安装成功回调:实例模式下用来刷新「已安装」列表。 */
  onInstalled?: () => void;
  /** 内容来源平台(modrinth / curseforge);决定走哪个 provider 取版本/安装。缺省 modrinth。 */
  provider?: "modrinth" | "curseforge";
}> = (props) => {
  const meta = () => KIND_META()[props.kind];
  const lockMode = () => !!props.lockedInstance;
  const provider = () => props.provider ?? "modrinth";
  // CurseForge 作者禁第三方分发时,后端把文件计入 report.blocked;此时不算成功,提示去网页手动下载。
  const warnIfBlocked = (n: number) => {
    if (n > 0) toast({ type: "warn", message: t("discover.blockedManual", { count: n }) });
    return n > 0;
  };

  // Esc 返回上一层(与列表/灯箱一致的导航直觉)。
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        props.onBack();
      }
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [installing, setInstalling] = createSignal(false);
  const [installingVersion, setInstallingVersion] = createSignal<string | null>(null);
  const [openAbout, setOpenAbout] = createSignal(true);

  // 实例列表来自全局 store(单一真相);锁定模式直接用 props.lockedInstance,不读它。
  const [project] = createResource(
    () => props.hit.id,
    (id) =>
      cached(`project|${provider()}|${id}`, () => api.modrinthProject(id)).catch((e) => {
        toast({ type: "error", message: t("discover.aboutLoadFailed", { error: String(e) }) });
        return null as ModrinthProject | null;
      }),
  );
  const [versions] = createResource(
    () => props.hit.id,
    (id) =>
      cached(`versions|${provider()}|${id}`, () => api.modrinthVersions(id, provider())).catch((e) => {
        toast({ type: "error", message: t("discover.versionsLoadFailed", { error: String(e) }) });
        return [] as ModrinthVersion[];
      }),
  );

  // 数据包逐存档生效:选好实例后还要选目标存档(saves/<world>/datapacks)。
  const isDatapack = () => props.kind === "datapack";
  const [worlds] = createResource(
    () => (isDatapack() ? selectedId() : false),
    (id) => api.instanceWorlds(activeRoot(), id as string),
  );
  const [world, setWorld] = createSignal<string | null>(null);
  createEffect(() => {
    if (!isDatapack()) return;
    const w = worlds() ?? [];
    if (w.length === 0) setWorld(null);
    else if (!world() || !w.some((x) => x.folder === world())) setWorld(w[0].folder);
  });
  const worldArg = () => (isDatapack() ? world() : null);

  const list = () => (props.lockedInstance ? [props.lockedInstance] : instances() ?? []);
  const versionList = () => versions() ?? [];
  const selectedInstance = () => list().find((inst) => inst.id === selectedId()) ?? null;
  const compatibleFor = (inst: InstanceSummary) => compatibleVersionsFor(versionList(), inst, props.kind);
  const canInstallTo = (inst: InstanceSummary) => {
    if (props.kind !== "mod") return true;
    if (inst.loader === "vanilla") return false;
    return versions.loading || versionList().length === 0 || compatibleFor(inst).length > 0;
  };
  const installableInstances = () => list().filter(canInstallTo);

  createEffect(() => {
    const current = selectedInstance();
    if (current && canInstallTo(current)) return;
    const preferred = installableInstances()[0] ?? list()[0];
    setSelectedId(preferred?.id ?? null);
  });

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
    if (!inst || installing() || installingVersion()) return;
    if (props.kind === "mod" && !versionMatches(version, inst, props.kind)) {
      toast({ type: "error", message: t("discover.incompatibleVersion") });
      return;
    }
    if (isDatapack() && !world()) {
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
    if (!inst || installing() || installingVersion()) return;
    if (!canInstallTo(inst)) {
      toast({ type: "error", message: t("discover.noCompatibleInstance") });
      return;
    }
    if (isDatapack() && !world()) {
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
    <div class="flex flex-col gap-[16px] px-[2px] pt-[4px] pb-[24px] overflow-y-auto">
      <button
        class="self-start bg-transparent border-none text-accent text-[14px] cursor-pointer py-[4px] px-0 transition-opacity duration-[var(--dur)] ease-app hover:opacity-70"
        onClick={props.onBack}
      >
        {t("discover.back")}
      </button>

      <div class="grid grid-cols-[minmax(0,1fr)_320px] gap-[18px] max-[980px]:grid-cols-1">
        <div class="min-w-0 flex flex-col gap-[16px]">
          <section class="flex flex-col gap-[12px]">
            <Show when={props.hit.gallery_url}>
              <img
                class="w-full max-h-[220px] object-cover rounded-none shadow-sunken"
                src={props.hit.gallery_url}
                alt=""
                width="960"
                height="540"
              />
            </Show>
            <div class="flex items-center gap-[14px]">
              <Show
                when={props.hit.icon_url}
                fallback={
                  <Panel variant="raised" class="w-[70px] h-[70px] flex items-center justify-center font-display text-[30px] text-strong bg-panel-2">
                    {(props.hit.title[0] ?? "?").toUpperCase()}
                  </Panel>
                }
              >
                <img
                  class="w-[70px] h-[70px] rounded-none object-cover flex-[0_0_auto] shadow-sunken"
                  src={props.hit.icon_url}
                  alt=""
                  width="70"
                  height="70"
                  style="image-rendering:pixelated"
                />
              </Show>
              <div class="min-w-0">
                <div class="text-[12px] font-semibold text-tag">{meta().title}</div>
                <Heading as="h1" size="page" class="whitespace-nowrap overflow-hidden text-ellipsis">{props.hit.title}</Heading>
                <div class="mt-[4px] text-[13px] text-sub">
                  by {props.hit.author} · ⬇ {props.hit.downloads.toLocaleString()}
                  <Show when={project()?.followers}>{" · ♥ " + project()!.followers.toLocaleString()}</Show>
                </div>
              </div>
            </div>
            <p class="m-0 text-[14px] leading-[1.7] text-sub">{props.hit.description}</p>
            <div class="flex flex-wrap gap-[6px]">
              <For each={props.hit.categories}>
                {(category) => <Tag>{category}</Tag>}
              </For>
            </div>
          </section>

          <section class="flex flex-col gap-[10px]">
            <Panel
              as="button"
              variant="raised"
              class="flex items-center justify-between border-none bg-panel-3 px-[14px] py-[11px] text-left cursor-pointer active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
              onClick={() => setOpenAbout((v) => !v)}
            >
              <Heading size="sub">{t("discover.projectAbout")}</Heading>
              <span class="text-muted">{openAbout() ? "⌃" : "⌄"}</span>
            </Panel>
            <Show when={openAbout()}>
              <Panel variant="sunken" class="bg-panel px-[14px] py-[12px]">
                <Show
                  when={!project.loading}
                  fallback={
                    <div class="flex items-center gap-[10px] text-muted text-[13px]">
                      <Spinner size={16} /> {t("discover.loadingAbout")}
                    </div>
                  }
                >
                  <Show
                    when={project()?.body?.trim()}
                    fallback={<div class="text-muted text-[13px]">{t("discover.noAboutBody")}</div>}
                  >
                    <div class="md text-[14px] leading-[1.75] text-sub" innerHTML={renderMarkdown(project()!.body)} />
                  </Show>
                </Show>
              </Panel>
            </Show>
          </section>

          <section class="flex flex-col gap-[8px]">
            <div class="flex items-center justify-between">
              <Heading size="sub">{t("discover.tabVersions")}</Heading>
              <span class="text-[12px] text-muted">{t("discover.versionsCount", { count: versionList().length })}</span>
            </div>
            <Show
              when={!versions.loading}
              fallback={
                <div class="flex items-center gap-[10px] text-muted text-[13px] py-[8px]">
                  <Spinner size={16} /> {t("discover.loadingVersions")}
                </div>
              }
            >
              <Show when={versionList().length > 0} fallback={<div class="text-muted text-[13px] py-[10px]">{t("discover.noVersionsDot")}</div>}>
                <div class="flex flex-col gap-[6px] max-h-[360px] overflow-y-auto">
                  <For each={versionList()}>
                    {(version) => {
                      const inst = () => selectedInstance();
                      const compatible = () => {
                        const i = inst();
                        return i ? versionMatches(version, i, props.kind) : false;
                      };
                      const busy = () => installing() || installingVersion() !== null;
                      // 仅 mod 的加载器/版本不匹配是硬性不可装(必崩);资源包/光影的版本差异
                      // 只是软提示(游戏内可带警告加载),不拦。
                      const blocked = () => props.kind === "mod" && !!inst() && !compatible();
                      return (
                        <Panel
                          variant="sunken"
                          class="flex items-center gap-[10px] bg-panel-2 px-[10px] py-[8px]"
                          classList={{ "opacity-55": !!inst() && !compatible() }}
                        >
                          <div class="min-w-0 flex-1">
                            <div class="text-[13px] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                              {version.version_number}
                              <span class="ml-[6px] text-[11px] font-medium text-muted">{typeLabel(version.version_type)}</span>
                            </div>
                            <div class="mt-[2px] text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                              {version.game_versions.slice(0, 5).join(", ")}
                              <Show when={version.loaders.length}>{" · " + version.loaders.join(" / ")}</Show>
                              {" · "}
                              {fmtDate(version.date_published)}
                              <Show when={version.file_size}>{" · " + fmtSize(version.file_size)}</Show>
                            </div>
                          </div>
                          <span class="text-[11px] text-muted whitespace-nowrap">⬇ {version.downloads.toLocaleString()}</span>
                          <button
                            class={ACCENT_BTN_COMPACT}
                            disabled={busy() || !inst() || blocked()}
                            title={!inst() ? t("discover.selectInstanceFirst") : blocked() ? t("discover.blockedTooltip") : ""}
                            onClick={() => installVersion(version)}
                          >
                            {installingVersion() === version.id ? t("discover.installing") : t("discover.install")}
                          </button>
                        </Panel>
                      );
                    }}
                  </For>
                </div>
              </Show>
            </Show>
          </section>
        </div>

        <aside class="flex flex-col gap-[12px]">
          {/* 全局模式才显示实例选择器;实例详情进入时目标已锁定。 */}
          <Show when={!lockMode()}>
            <Panel as="section" variant="sunken" class="bg-panel px-[14px] py-[14px]">
              <Heading size="sub" class="mb-[10px]">{t("discover.installToInstance")}</Heading>
              <Show
                when={!instances.loading}
                fallback={
                  <div class="flex items-center gap-[10px] text-muted text-[13px]">
                    <Spinner size={16} /> {t("discover.loadingInstances")}
                  </div>
                }
              >
                <Show
                  when={list().length > 0}
                  fallback={<div class="text-[13px] leading-[1.7] text-muted">{t("discover.noInstances")}</div>}
                >
                  <div class="flex flex-col gap-[6px] max-h-[310px] overflow-y-auto">
                    <For each={list()}>
                      {(inst) => {
                        const disabled = !canInstallTo(inst);
                        const compatCount = () => compatibleFor(inst).length;
                        return (
                          <button
                            class="flex w-full items-center gap-[9px] rounded-none bg-panel-2 px-[9px] py-[8px] text-left cursor-pointer shadow-sunken transition-[box-shadow] duration-[var(--dur)] ease-app disabled:cursor-not-allowed disabled:opacity-50"
                            classList={{
                              "!bg-accent !shadow-raised": selectedId() === inst.id,
                            }}
                            disabled={disabled}
                            onClick={() => setSelectedId(inst.id)}
                          >
                            <span
                              class={`w-[30px] h-[30px] flex-[0_0_30px] rounded-none grid place-items-center bg-accent text-accent-text text-[13px] font-bold shadow-raised ${LOADER_BADGE_TINT}`}
                              data-loader={inst.loader}
                            >
                              {(inst.name || inst.id)[0]?.toUpperCase()}
                            </span>
                            <span class="min-w-0 flex-1">
                              <span
                                class="block text-[13px] font-semibold whitespace-nowrap overflow-hidden text-ellipsis"
                                classList={{ "text-accent-text": selectedId() === inst.id, "text-fg": selectedId() !== inst.id }}
                              >
                                {inst.name || inst.id}
                              </span>
                              <span
                                class="block text-[11px] whitespace-nowrap overflow-hidden text-ellipsis"
                                classList={{ "text-accent-text": selectedId() === inst.id, "text-muted": selectedId() !== inst.id }}
                              >
                                {inst.mc_version} · {loaderLabel(inst.loader)}
                                <Show when={props.kind === "mod" && inst.loader !== "vanilla"}>
                                  {" · " + t("discover.matchingVersions", { count: compatCount() })}
                                </Show>
                              </span>
                            </span>
                          </button>
                        );
                      }}
                    </For>
                  </div>
                </Show>
              </Show>
            </Panel>
          </Show>

          <Panel as="section" variant="sunken" class="bg-panel px-[14px] py-[14px]">
            <Show when={selectedInstance()} fallback={<div class="text-[13px] text-muted">{t("discover.selectInstanceToInstall")}</div>}>
              {(inst) => (
                <div class="flex flex-col gap-[10px]">
                  <div>
                    <div class="text-[12px] text-muted">{t("discover.target")}</div>
                    <Heading size="sub" class="mt-[2px]">{inst().name || inst().id}</Heading>
                    <div class="mt-[2px] text-[12px] text-muted">
                      Minecraft {inst().mc_version} · {loaderLabel(inst().loader)}
                    </div>
                  </div>

                  {/* 数据包目标存档:数据包逐存档生效,必须先选一个存档。 */}
                  <Show when={isDatapack()}>
                    <Show
                      when={(worlds() ?? []).length > 0}
                      fallback={
                        <div class="text-[12px] leading-[1.7] text-muted">
                          {t("discover.datapackNoWorlds")}
                        </div>
                      }
                    >
                      <label class="flex items-center gap-[8px] text-[12px] text-muted">
                        <span class="shrink-0">{t("discover.targetWorld")}</span>
                        <Select
                          class="flex-1 !min-w-0"
                          value={world() ?? ""}
                          onChange={(v) => setWorld(v)}
                          options={(worlds() ?? []).map((w) => ({ value: w.folder, label: w.name || w.folder }))}
                        />
                      </label>
                    </Show>
                  </Show>

                  <button
                    class={ACCENT_BTN}
                    disabled={installing() || installingVersion() !== null || !canInstallTo(inst()) || (isDatapack() && !world())}
                    onClick={installLatest}
                  >
                    {installing() ? t("discover.installing") : t("discover.installLatest", { kind: meta().title })}
                  </button>
                  <Show when={props.kind === "mod" && inst().loader === "vanilla"}>
                    <div class="text-[12px] leading-[1.6] text-muted">{t("discover.vanillaNoModLoader")}</div>
                  </Show>
                  <Show when={props.kind === "mod" && inst().loader !== "vanilla" && !versions.loading && compatibleFor(inst()).length === 0}>
                    <div class="text-[12px] leading-[1.6] text-muted">{t("discover.noMatchingFile")}</div>
                  </Show>
                  <Show when={links().length}>
                    <div class="flex flex-wrap gap-[6px] pt-[4px]">
                      <For each={links()}>
                        {(link) => (
                          <button
                            class="h-[28px] rounded-none bg-panel-3 px-[10px] text-[12px] text-tag cursor-pointer shadow-raised active:shadow-pressed transition-[box-shadow] duration-[var(--dur)] ease-app"
                            onClick={() => shellOpen(link.url)}
                          >
                            {link.label} ↗
                          </button>
                        )}
                      </For>
                    </div>
                  </Show>
                </div>
              )}
            </Show>
          </Panel>
        </aside>
      </div>
    </div>
  );
};

export default ProjectInstallDetail;
