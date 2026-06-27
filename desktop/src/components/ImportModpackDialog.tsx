import { Component, createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Dialog } from "./Dialog";
import { Icon } from "./Icon";
import { Spinner } from "./Spinner";
import { Button } from "./Button";
import { Panel } from "./Panel";
import { Tag } from "./Tag";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { api, onInstallProgress } from "../ipc/api";
import { t } from "../i18n";
import type { ImportOutcome } from "../ipc/types";

/**
 * ImportModpackDialog —— 导入整合包的统一入口(两套布局共用)。把之前「点按钮直接弹系统文件框」
 * 的隐式流程显式化:展示**支持的格式**(Modrinth / CurseForge / MultiMC / MCBBS)、**拖入提示**
 * 与**导入须知**,并同时支持「拖入文件」与「点击选择」两种方式。
 *
 * Tauri 启用原生文件拖放,HTML5 ondrop 不触发,故走 webview 的 onDragDropEvent(仅在打开时监听)。
 */

// label/tip 必须在渲染期(getter)调 t(),放模块常量会冻结语言(见 i18n 备忘)。
const FORMAT_EXT: { mrpack: string; zip: string } = { mrpack: ".mrpack", zip: ".zip" };

export const ImportModpackDialog: Component<{
  open: boolean;
  root: string;
  onClose: () => void;
  onImported: (out: ImportOutcome) => void;
  /** 打开时若带一个文件路径(从页面拖入),自动开始导入它(每个 path 仅触发一次)。 */
  initialPath?: string | null;
}> = (props) => {
  const [importing, setImporting] = createSignal(false);
  const [progress, setProgress] = createSignal("");
  const [dragOver, setDragOver] = createSignal(false);
  const [handledPath, setHandledPath] = createSignal<string | null>(null);

  const formats = () => [
    { ext: FORMAT_EXT.mrpack, label: t("components.import.fmtModrinth") },
    { ext: FORMAT_EXT.zip, label: t("components.import.fmtCurseforge") },
    { ext: FORMAT_EXT.zip, label: t("components.import.fmtMultimc") },
    { ext: FORMAT_EXT.zip, label: t("components.import.fmtMcbbs") },
    { ext: "/", label: t("components.import.fmtDir") },
  ];
  const tips = () => [
    t("components.import.tipFormats"),
    t("components.import.tipCurseforge"),
    t("components.import.tipProgress"),
    t("components.import.tipPreserve"),
  ];

  async function runImport(path: string) {
    if (importing()) return;
    setImporting(true);
    setProgress("");
    const off = onInstallProgress((p) =>
      setProgress(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage),
    );
    try {
      const out = await api.importModpack(props.root, path, null);
      props.onImported(out);
      props.onClose();
    } catch (e) {
      toast({ type: "error", message: t("components.import.failed", { err: String(e) }) });
    } finally {
      off();
      setImporting(false);
      setProgress("");
    }
  }

  async function pick() {
    if (importing()) return;
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: t("components.import.filter"), extensions: ["mrpack", "zip"] }],
    });
    if (typeof picked === "string") void runImport(picked);
  }

  // 选未解压的 MultiMC/Prism 实例目录(磁盘上 Prism 实例本就是目录)。引擎按目录后端导入。
  async function pickFolder() {
    if (importing()) return;
    const picked = await openDialog({ directory: true, multiple: false });
    if (typeof picked === "string") void runImport(picked);
  }

  // 从页面拖入打开:对带入的 path 自动开始导入,每个 path 仅触发一次;关闭后复位。
  createEffect(() => {
    const path = props.initialPath;
    if (props.open && path && handledPath() !== path) {
      setHandledPath(path);
      void runImport(path);
    } else if (!props.open && handledPath() !== null) {
      setHandledPath(null);
    }
  });

  // 原生文件拖放:仅在弹窗打开时监听;拖入非 .mrpack/.zip 提示,多个只取第一个。
  createEffect(() => {
    if (!props.open) {
      setDragOver(false);
      return;
    }
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      if (!props.open) return;
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") setDragOver(true);
      else if (p.type === "leave") setDragOver(false);
      else if (p.type === "drop") {
        setDragOver(false);
        const first = p.paths.find((x) => /\.(mrpack|zip)$/i.test(x));
        if (!first) {
          toast({ type: "info", message: t("components.import.unsupported") });
          return;
        }
        if (p.paths.length > 1) toast({ type: "info", message: t("components.import.onlyFirst") });
        void runImport(first);
      }
    });
    onCleanup(() => void unlisten.then((f) => f()));
  });

  return (
    <Dialog
      open={props.open}
      onClose={() => !importing() && props.onClose()}
      label={t("components.import.title")}
      contentClass="w-[460px] max-w-[calc(100vw-48px)]"
    >
      <div class="flex flex-col gap-[16px] p-[20px]">
        <Heading size="sub">{t("components.import.title")}</Heading>

        {/* 拖入区(点击选择 / 拖入文件)。拖到窗口时高亮。 */}
        <button
          type="button"
          disabled={importing()}
          onClick={() => void pick()}
          class="flex flex-col items-center justify-center gap-[10px] rounded-none border-2 border-dashed px-[20px] py-[28px] text-center cursor-pointer transition-colors duration-150 disabled:cursor-default"
          classList={{
            "border-accent bg-panel-2": dragOver(),
            "border-titlebar bg-sidebar hover:bg-panel-2": !dragOver(),
          }}
        >
          <Show when={!importing()} fallback={<Spinner />}>
            <Icon name="download" size={26} class="text-accent" />
          </Show>
          <div class="text-[13px] font-semibold text-fg">
            {importing() ? t("components.import.importing") : t("components.import.dropHint")}
          </div>
          <div class="text-[11px] text-muted truncate max-w-full">
            {importing()
              ? progress() || t("components.import.importing")
              : t("components.import.clickHint")}
          </div>
        </button>

        {/* 支持的格式 */}
        <div>
          <div class="text-[12px] font-semibold text-sub mb-[6px]">
            {t("components.import.formatsTitle")}
          </div>
          <div class="flex flex-col gap-[5px]">
            <For each={formats()}>
              {(f) => (
                <div class="flex items-center gap-[8px] text-[12px]">
                  <Tag class="font-pixel">{f.ext}</Tag>
                  <span class="text-fg">{f.label}</span>
                </div>
              )}
            </For>
          </div>
        </div>

        {/* 导入须知 */}
        <Panel variant="input" class="px-[12px] py-[10px]">
          <div class="flex items-center gap-[6px] text-[12px] font-semibold text-sub mb-[5px]">
            <Icon name="info" size={14} /> {t("components.import.tipsTitle")}
          </div>
          <ul class="m-0 pl-[16px] flex flex-col gap-[3px] text-[11px] leading-[1.6] text-muted">
            <For each={tips()}>{(tip) => <li>{tip}</li>}</For>
          </ul>
        </Panel>

        <div class="flex justify-end gap-[10px]">
          <Button variant="ghost" disabled={importing()} onClick={() => props.onClose()}>
            {t("components.import.close")}
          </Button>
          <Button variant="ghost" disabled={importing()} onClick={() => void pickFolder()}>
            {t("components.import.chooseFolder")}
          </Button>
          <Button variant="primary" disabled={importing()} onClick={() => void pick()}>
            {t("components.import.choose")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
};

export default ImportModpackDialog;
