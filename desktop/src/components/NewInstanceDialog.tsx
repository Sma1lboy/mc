import { useEffect, useMemo, useState } from "react";
import { Dialog } from "./Dialog";
import { Select } from "./Select";
import { Segmented } from "./Segmented";
import { Spinner } from "./Spinner";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { useAsync } from "../util/useAsync";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot } from "../store";
import { openAgentChat, instancePrompt } from "../agent/chatStore";
import { t, useLang } from "../i18n";

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

export function NewInstanceDialog(props: {
  open: boolean;
  onClose: () => void;
  onCreated?: (id: string) => void;
}) {
  const lang = useLang();
  const [name, setName] = useState("");
  const [mcVersion, setMcVersion] = useState("");
  const [loader, setLoader] = useState("vanilla");
  const [loaderVersion, setLoaderVersion] = useState("");
  const [creating, setCreating] = useState(false);
  const [stage, setStage] = useState("");

  const { data: versions, loading: versionsLoading } = useAsync(
    () => api.listVersions(false).catch(() => [] as { id: string }[]),
    [],
  );
  const versionOptions = useMemo(
    () => (versions ?? []).map((v) => ({ label: v.id, value: v.id })),
    [versions],
  );

  // forge/neoforge 必须选具体构建号;fabric/quilt 版本可选(留空=最新);vanilla 无 loader 版本。
  const needsLoaderVersion = loader === "forge" || loader === "neoforge";
  const supportsLoaderVersion = loader !== "vanilla";

  // 可用 loader 版本由 daemon 拉真实元数据(forge/neoforge maven、fabric/quilt meta),
  // 免去手填。仅在选了 loader + MC 版本时请求;失败/为空时回退,绝不卡住用户。
  const { data: loaderVersions, loading: loaderVersionsLoading } = useAsync<string[]>(
    async () => {
      if (!supportsLoaderVersion || !mcVersion) return [];
      try {
        return await api.listLoaderVersions(loader, mcVersion);
      } catch {
        return [];
      }
    },
    [loader, mcVersion, supportsLoaderVersion],
  );
  // 可选(fabric/quilt)在列表前加「最新(推荐)」哨兵(value 空 → 后端选最新)。
  const loaderVersionOptions = useMemo(() => {
    const list = (loaderVersions ?? []).map((v) => ({ label: v, value: v }));
    return needsLoaderVersion
      ? list
      : [{ label: t("components.newInstance.latestRecommended"), value: "" }, ...list];
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaderVersions, needsLoaderVersion, lang]);
  // 列表到手即预选最新(第一个);仅对必填(forge/neoforge)生效,可选 loader 默认留空=最新。
  useEffect(() => {
    if (!needsLoaderVersion) return;
    if (loaderVersions && loaderVersions.length > 0 && !loaderVersions.includes(loaderVersion)) {
      setLoaderVersion(loaderVersions[0]);
    }
  }, [loaderVersions, needsLoaderVersion, loaderVersion]);

  const canCreate =
    !creating &&
    name.trim() !== "" &&
    mcVersion !== "" &&
    (!needsLoaderVersion || loaderVersion.trim() !== "");

  // 关掉对话框,带当前所选版本 / 加载器打开助手(未选则省略;vanilla 无加载器视作未选)。
  function askAgent() {
    props.onClose();
    openAgentChat(instancePrompt(mcVersion || null, loader !== "vanilla" ? loader : null), {
      mode: "build",
    });
  }

  async function create() {
    if (!canCreate) return;
    setCreating(true);
    setStage(t("components.newInstance.preparing"));
    const unlisten = onInstallProgress((p) =>
      setStage(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage),
    );
    try {
      const id = await api.createInstance(
        activeRoot(),
        name.trim(),
        mcVersion,
        loader,
        // 非 vanilla 时传所选版本;空串(fabric/quilt 选「最新」或 vanilla)→ null=最新。
        loaderVersion.trim() || null,
      );
      toast({ type: "success", message: t("components.newInstance.created", { name: name.trim() }) });
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
      onClose={() => !creating && props.onClose()}
      label={t("components.newInstance.title")}
      contentClass="w-[440px] max-w-[calc(100vw-48px)]"
    >
      <div className="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub">{t("components.newInstance.title")}</Heading>

        <label className="flex flex-col gap-[5px]">
          <span className="text-[12px] text-sub">{t("components.newInstance.name")}</span>
          <input
            className={FIELD}
            name="instanceName"
            autoComplete="off"
            spellCheck={false}
            placeholder={t("components.newInstance.namePlaceholder")}
            value={name}
            onChange={(e) => setName(e.currentTarget.value)}
            disabled={creating}
          />
        </label>

        <label className="flex flex-col gap-[5px]">
          <span className="text-[12px] text-sub">{t("components.newInstance.mcVersion")}</span>
          <Select
            className="w-full"
            value={mcVersion}
            onChange={setMcVersion}
            options={versionOptions}
            placeholder={versionsLoading ? t("components.newInstance.loadingVersions") : t("components.newInstance.selectVersion")}
          />
        </label>

        <label className="flex flex-col gap-[5px]">
          <span className="text-[12px] text-sub">{t("components.newInstance.loader")}</span>
          {/* 切换 loader 时清掉上一个 loader 的版本选择,避免把 forge build 号带进 fabric。 */}
          <Segmented
            className="self-start"
            ariaLabel={t("components.newInstance.loader")}
            value={loader}
            onChange={(v) => {
              setLoader(v);
              setLoaderVersion("");
            }}
            options={LOADERS().map((l) => ({ value: l.value, label: l.label }))}
          />
        </label>

        {supportsLoaderVersion && (
          <label className="flex flex-col gap-[5px]">
            <span className="text-[12px] text-sub">
              {loader === "forge"
                ? t("components.newInstance.forgeVersion")
                : loader === "neoforge"
                  ? t("components.newInstance.neoforgeVersion")
                  : loader === "fabric"
                    ? t("components.newInstance.fabricVersionOptional")
                    : t("components.newInstance.quiltVersionOptional")}
            </span>
            {!loaderVersionsLoading && loaderVersionOptions.length > 0 ? (
              <Select
                className="w-full"
                value={loaderVersion}
                onChange={setLoaderVersion}
                options={loaderVersionOptions}
                placeholder={t("components.newInstance.selectVersion")}
              />
            ) : !loaderVersionsLoading ? (
              /* 拉取失败 / 该版本无可用构建 → 退回手填,绝不卡住用户。 */
              <input
                className={FIELD}
                name="loaderVersion"
                autoComplete="off"
                spellCheck={false}
                placeholder={loader === "forge" ? t("components.newInstance.forgePlaceholder") : t("components.newInstance.neoforgePlaceholder")}
                value={loaderVersion}
                onChange={(e) => setLoaderVersion(e.currentTarget.value)}
                disabled={creating}
              />
            ) : (
              <div className="flex items-center gap-[8px] h-[36px] px-[12px] text-[12px] text-muted">
                <Spinner size={14} />
                <span>{t("components.newInstance.loadingAvailable")}</span>
              </div>
            )}
          </label>
        )}

        {creating && (
          <div className="flex items-center gap-[10px] text-[12px] text-muted">
            <Spinner size={16} />
            <span>{stage || t("components.newInstance.creating")}</span>
          </div>
        )}

        {/* 让 AI 生成整合包:关掉对话框、带上下文打开助手(不自动发送)。 */}
        <div className="pt-[12px] border-t border-titlebar flex justify-center">
          <Button variant="ghost" disabled={creating} onClick={askAgent}>
            {t("agent.newInstanceCta")}
          </Button>
        </div>

        <div className="flex justify-end gap-[10px] mt-[4px]">
          <Button variant="ghost" onClick={props.onClose} disabled={creating}>
            {t("components.newInstance.cancel")}
          </Button>
          <Button variant="primary" onClick={create} disabled={!canCreate}>
            {creating ? t("components.newInstance.creating") : t("components.newInstance.create")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

export default NewInstanceDialog;
