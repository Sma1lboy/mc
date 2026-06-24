import {
  Component,
  createSignal,
  createResource,
  createMemo,
  createEffect,
  Show,
} from "solid-js";
import { Dialog } from "./Dialog";
import { Select } from "./Select";
import { Segmented } from "./Segmented";
import { Spinner } from "./Spinner";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot } from "../store";
import { t } from "../i18n";

/**
 * NewInstanceDialog —— 从零新建实例:名称 + MC 版本 + 加载器(forge/neoforge 再要版本)。
 * 调 daemon 的 create_instance(装核心 → 命名实例),进度走 install://progress。
 */

const LOADERS = () => [
  { label: t("components.newInstance.loaderVanilla"), value: "vanilla" },
  { label: t("components.newInstance.loaderFabric"), value: "fabric" },
  { label: t("components.newInstance.loaderQuilt"), value: "quilt" },
  { label: t("components.newInstance.loaderForge"), value: "forge" },
  { label: t("components.newInstance.loaderNeoforge"), value: "neoforge" },
];

const FIELD =
  "h-[36px] px-[12px] rounded-none bg-sidebar shadow-input text-fg text-[13px] " +
  "placeholder:text-faint transition-[box-shadow] duration-150 focus-visible:outline-none " +
  "focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50";

export const NewInstanceDialog: Component<{
  open: boolean;
  onClose: () => void;
  onCreated?: (id: string) => void;
}> = (props) => {
  const [name, setName] = createSignal("");
  const [mcVersion, setMcVersion] = createSignal("");
  const [loader, setLoader] = createSignal("vanilla");
  const [loaderVersion, setLoaderVersion] = createSignal("");
  const [creating, setCreating] = createSignal(false);
  const [stage, setStage] = createSignal("");

  const [versions] = createResource(() =>
    api.listVersions(false).catch(() => [] as { id: string }[]),
  );
  const versionOptions = createMemo(() =>
    (versions() ?? []).map((v) => ({ label: v.id, value: v.id })),
  );

  // forge/neoforge 必须选具体构建号;fabric/quilt 版本可选(留空=最新);vanilla 无 loader 版本。
  const needsLoaderVersion = () => loader() === "forge" || loader() === "neoforge";
  const supportsLoaderVersion = () => loader() !== "vanilla";

  // 可用 loader 版本由 daemon 拉真实元数据(forge/neoforge maven、fabric/quilt meta),
  // 免去手填。仅在选了 loader + MC 版本时请求;失败/为空时回退,绝不卡住用户。
  const [loaderVersions] = createResource(
    () => (supportsLoaderVersion() && mcVersion() ? ([loader(), mcVersion()] as const) : null),
    async ([ld, mc]) => {
      try {
        return await api.listLoaderVersions(ld, mc);
      } catch {
        return [] as string[];
      }
    },
  );
  // 可选(fabric/quilt)在列表前加「最新(推荐)」哨兵(value 空 → 后端选最新)。
  const loaderVersionOptions = createMemo(() => {
    const list = (loaderVersions() ?? []).map((v) => ({ label: v, value: v }));
    return needsLoaderVersion() ? list : [{ label: t("components.newInstance.latestRecommended"), value: "" }, ...list];
  });
  // 列表到手即预选最新(第一个);仅对必填(forge/neoforge)生效,可选 loader 默认留空=最新。
  createEffect(() => {
    const list = loaderVersions();
    if (!needsLoaderVersion()) return;
    if (list && list.length > 0 && !list.includes(loaderVersion())) {
      setLoaderVersion(list[0]);
    }
  });

  const canCreate = () =>
    !creating() &&
    name().trim() !== "" &&
    mcVersion() !== "" &&
    (!needsLoaderVersion() || loaderVersion().trim() !== "");

  async function create() {
    if (!canCreate()) return;
    setCreating(true);
    setStage(t("components.newInstance.preparing"));
    const unlisten = onInstallProgress((p) =>
      setStage(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage),
    );
    try {
      const id = await api.createInstance(
        activeRoot(),
        name().trim(),
        mcVersion(),
        loader(),
        // 非 vanilla 时传所选版本;空串(fabric/quilt 选「最新」或 vanilla)→ null=最新。
        loaderVersion().trim() || null,
      );
      toast({ type: "success", message: t("components.newInstance.created", { name: name().trim() }) });
      props.onCreated?.(id);
      props.onClose();
    } catch (e) {
      toast({ type: "error", message: t("components.newInstance.createFailed", { error: String(e) }) });
    } finally {
      unlisten();
      setCreating(false);
      setStage("");
    }
  }

  return (
    <Dialog
      open={props.open}
      onClose={() => !creating() && props.onClose()}
      label={t("components.newInstance.title")}
      contentClass="w-[440px] max-w-[calc(100vw-48px)]"
    >
      <div class="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub">{t("components.newInstance.title")}</Heading>

        <label class="flex flex-col gap-[5px]">
          <span class="text-[12px] text-sub">{t("components.newInstance.name")}</span>
          <input
            class={FIELD}
            name="instanceName"
            autocomplete="off"
            spellcheck={false}
            placeholder={t("components.newInstance.namePlaceholder")}
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            disabled={creating()}
          />
        </label>

        <label class="flex flex-col gap-[5px]">
          <span class="text-[12px] text-sub">{t("components.newInstance.mcVersion")}</span>
          <Select
            class="w-full"
            value={mcVersion()}
            onChange={setMcVersion}
            options={versionOptions()}
            placeholder={versions.loading ? t("components.newInstance.loadingVersions") : t("components.newInstance.selectVersion")}
          />
        </label>

        <label class="flex flex-col gap-[5px]">
          <span class="text-[12px] text-sub">{t("components.newInstance.loader")}</span>
          {/* 切换 loader 时清掉上一个 loader 的版本选择,避免把 forge build 号带进 fabric。 */}
          <Segmented
            class="self-start"
            ariaLabel={t("components.newInstance.loader")}
            value={loader()}
            onChange={(v) => {
              setLoader(v);
              setLoaderVersion("");
            }}
            options={LOADERS().map((l) => ({ value: l.value, label: l.label }))}
          />
        </label>

        <Show when={supportsLoaderVersion()}>
          <label class="flex flex-col gap-[5px]">
            <span class="text-[12px] text-sub">
              {loader() === "forge"
                ? t("components.newInstance.forgeVersion")
                : loader() === "neoforge"
                  ? t("components.newInstance.neoforgeVersion")
                  : loader() === "fabric"
                    ? t("components.newInstance.fabricVersionOptional")
                    : t("components.newInstance.quiltVersionOptional")}
            </span>
            <Show
              when={!loaderVersions.loading && loaderVersionOptions().length > 0}
              fallback={
                <Show
                  when={!loaderVersions.loading}
                  fallback={
                    <div class="flex items-center gap-[8px] h-[36px] px-[12px] text-[12px] text-muted">
                      <Spinner size={14} />
                      <span>{t("components.newInstance.loadingAvailable")}</span>
                    </div>
                  }
                >
                  {/* 拉取失败 / 该版本无可用构建 → 退回手填,绝不卡住用户。 */}
                  <input
                    class={FIELD}
                    name="loaderVersion"
                    autocomplete="off"
                    spellcheck={false}
                    placeholder={loader() === "forge" ? t("components.newInstance.forgePlaceholder") : t("components.newInstance.neoforgePlaceholder")}
                    value={loaderVersion()}
                    onInput={(e) => setLoaderVersion(e.currentTarget.value)}
                    disabled={creating()}
                  />
                </Show>
              }
            >
              <Select
                class="w-full"
                value={loaderVersion()}
                onChange={setLoaderVersion}
                options={loaderVersionOptions()}
                placeholder={t("components.newInstance.selectVersion")}
              />
            </Show>
          </label>
        </Show>

        <Show when={creating()}>
          <div class="flex items-center gap-[10px] text-[12px] text-muted">
            <Spinner size={16} />
            <span>{stage() || t("components.newInstance.creating")}</span>
          </div>
        </Show>

        <div class="flex justify-end gap-[10px] mt-[4px]">
          <Button variant="ghost" onClick={props.onClose} disabled={creating()}>
            {t("components.newInstance.cancel")}
          </Button>
          <Button variant="primary" onClick={create} disabled={!canCreate()}>
            {creating() ? t("components.newInstance.creating") : t("components.newInstance.create")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
};

export default NewInstanceDialog;
