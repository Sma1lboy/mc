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
import { Spinner } from "./Spinner";
import { toast } from "./Toast";
import { api, onInstallProgress } from "../ipc/api";
import { activeRoot } from "../store";

/**
 * NewInstanceDialog —— 从零新建实例:名称 + MC 版本 + 加载器(forge/neoforge 再要版本)。
 * 调 daemon 的 create_instance(装核心 → 命名实例),进度走 install://progress。
 */

const LOADERS = [
  { label: "原版 (Vanilla)", value: "vanilla" },
  { label: "Fabric", value: "fabric" },
  { label: "Quilt", value: "quilt" },
  { label: "Forge", value: "forge" },
  { label: "NeoForge", value: "neoforge" },
];

const FIELD =
  "h-[36px] px-[12px] rounded-ctl border border-glass-border glass-input text-fg text-[13px] " +
  "transition-[border-color,box-shadow] duration-150 focus-visible:outline-none " +
  "focus-visible:border-a-4 focus-visible:ring-2 focus-visible:ring-a-5/25 disabled:opacity-50";

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

  const needsLoaderVersion = () => loader() === "forge" || loader() === "neoforge";

  // Forge/NeoForge 的可用构建号由 daemon 拉真实 maven 元数据,免去手填精确 build。
  // 仅在选了 loader + MC 版本时请求;失败/为空时下面回退到手填输入框,绝不卡住用户。
  const [loaderVersions] = createResource(
    () => (needsLoaderVersion() && mcVersion() ? ([loader(), mcVersion()] as const) : null),
    async ([ld, mc]) => {
      try {
        return await api.listLoaderVersions(ld, mc);
      } catch {
        return [] as string[];
      }
    },
  );
  const loaderVersionOptions = createMemo(() =>
    (loaderVersions() ?? []).map((v) => ({ label: v, value: v })),
  );
  // 列表到手即预选最新(第一个);用户未手动改过时随版本变化刷新默认值。
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
    setStage("准备…");
    const unlisten = onInstallProgress((p) =>
      setStage(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage),
    );
    try {
      const id = await api.createInstance(
        activeRoot(),
        name().trim(),
        mcVersion(),
        loader(),
        needsLoaderVersion() ? loaderVersion().trim() : null,
      );
      toast({ type: "success", message: `已创建实例「${name().trim()}」` });
      props.onCreated?.(id);
      props.onClose();
    } catch (e) {
      toast({ type: "error", message: `创建失败:${e}` });
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
      label="新建实例"
      contentClass="w-[440px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden"
    >
      <div class="p-[20px] flex flex-col gap-[14px]">
        <div class="text-[16px] font-bold text-fg">新建实例</div>

        <label class="flex flex-col gap-[5px]">
          <span class="text-[12px] text-dim">名称</span>
          <input
            class={FIELD}
            name="instanceName"
            autocomplete="off"
            spellcheck={false}
            placeholder="例如 生存整合包…"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            disabled={creating()}
          />
        </label>

        <label class="flex flex-col gap-[5px]">
          <span class="text-[12px] text-dim">Minecraft 版本</span>
          <Select
            value={mcVersion()}
            onChange={setMcVersion}
            options={versionOptions()}
            placeholder={versions.loading ? "加载版本中…" : "选择版本"}
          />
        </label>

        <label class="flex flex-col gap-[5px]">
          <span class="text-[12px] text-dim">加载器</span>
          <Select value={loader()} onChange={setLoader} options={LOADERS} />
        </label>

        <Show when={needsLoaderVersion()}>
          <label class="flex flex-col gap-[5px]">
            <span class="text-[12px] text-dim">
              {loader() === "forge" ? "Forge 版本" : "NeoForge 版本"}
            </span>
            <Show
              when={!loaderVersions.loading && loaderVersionOptions().length > 0}
              fallback={
                <Show
                  when={!loaderVersions.loading}
                  fallback={
                    <div class="flex items-center gap-[8px] h-[36px] px-[12px] text-[12px] text-dim">
                      <Spinner size={14} />
                      <span>加载可用版本中…</span>
                    </div>
                  }
                >
                  {/* 拉取失败 / 该版本无可用构建 → 退回手填,绝不卡住用户。 */}
                  <input
                    class={FIELD}
                    name="loaderVersion"
                    autocomplete="off"
                    spellcheck={false}
                    placeholder={loader() === "forge" ? "例如 47.2.0…" : "例如 20.4.237…"}
                    value={loaderVersion()}
                    onInput={(e) => setLoaderVersion(e.currentTarget.value)}
                    disabled={creating()}
                  />
                </Show>
              }
            >
              <Select
                value={loaderVersion()}
                onChange={setLoaderVersion}
                options={loaderVersionOptions()}
                placeholder="选择版本"
              />
            </Show>
          </label>
        </Show>

        <Show when={creating()}>
          <div class="flex items-center gap-[10px] text-[12px] text-dim">
            <Spinner size={16} />
            <span>{stage() || "创建中…"}</span>
          </div>
        </Show>

        <div class="flex justify-end gap-[10px] mt-[4px]">
          <button
            class="h-[34px] px-[16px] border border-glass-border rounded-ctl bg-glass-card text-fg text-[13px] cursor-pointer transition-colors duration-150 hover:bg-glass-hover disabled:opacity-50"
            onClick={props.onClose}
            disabled={creating()}
          >
            取消
          </button>
          <button
            class="h-[34px] px-[16px] border-none rounded-ctl bg-a-4 text-white text-[13px] font-semibold cursor-pointer transition-colors duration-150 hover:bg-a-5 disabled:opacity-50 disabled:cursor-not-allowed"
            onClick={create}
            disabled={!canCreate()}
          >
            {creating() ? "创建中…" : "创建"}
          </button>
        </div>
      </div>
    </Dialog>
  );
};

export default NewInstanceDialog;
