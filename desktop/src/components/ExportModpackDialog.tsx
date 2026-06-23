import { Component, createSignal, Show } from "solid-js";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Dialog } from "./Dialog";
import { Select } from "./Select";
import { toast } from "./Toast";
import { ACCENT_BTN } from "./styles";
import { api } from "../ipc/api";
import { t } from "../i18n";

/**
 * ExportModpackDialog —— 导出实例为整合包/清单的统一入口(两套布局共用)。
 *
 * 之前 UI 只暴露 Modrinth `.mrpack` 一种,而后端 `export_modpack` 早就支持 CurseForge `.zip`
 * 与多种 mod 清单(md/html/json/csv/txt)—— 这里把这些隐藏能力摊给用户选。选好格式 → 系统保存框
 * 选路径 → 导出。
 */

/** 导出所需的最小实例信息(InstanceRowData / InstanceSummary 都兼容)。 */
export interface ExportInstanceRef {
  id: string;
  name: string;
  mc_version: string;
  loader?: string | null;
  loader_version?: string | null;
}

type Fmt = "modrinth" | "curseforge" | "modlist";

export const ExportModpackDialog: Component<{
  open: boolean;
  root: string;
  instance: ExportInstanceRef | null;
  onClose: () => void;
}> = (props) => {
  const [fmt, setFmt] = createSignal<Fmt>("modrinth");
  const [sub, setSub] = createSignal("md");
  const [busy, setBusy] = createSignal(false);

  // 选定格式 → 文件扩展名 + export_modpack 的 target 串。
  const ext = () => (fmt() === "modrinth" ? "mrpack" : fmt() === "curseforge" ? "zip" : sub());
  const target = () => (fmt() === "modlist" ? `modlist:${sub()}` : fmt());

  const formatOptions = () => [
    { value: "modrinth", label: t("components.export.fmtModrinth") },
    { value: "curseforge", label: t("components.export.fmtCurseforge") },
    { value: "modlist", label: t("components.export.fmtModlist") },
  ];
  const subOptions = () => [
    { value: "md", label: t("components.export.subMd") },
    { value: "html", label: t("components.export.subHtml") },
    { value: "json", label: t("components.export.subJson") },
    { value: "csv", label: t("components.export.subCsv") },
    { value: "txt", label: t("components.export.subTxt") },
  ];
  const hint = () =>
    fmt() === "modrinth"
      ? t("components.export.hintModrinth")
      : fmt() === "curseforge"
        ? t("components.export.hintCurseforge")
        : t("components.export.hintModlist");

  async function doExport() {
    const inst = props.instance;
    if (!inst || busy()) return;
    const name = inst.name || inst.id;
    const dest = await saveDialog({
      title: t("components.export.title"),
      defaultPath: `${name}.${ext()}`,
      filters: [{ name: t("components.export.filter"), extensions: [ext()] }],
    }).catch(() => null);
    if (!dest) return; // 用户取消
    setBusy(true);
    try {
      const out = await api.exportModpack({
        root: props.root,
        instanceId: inst.id,
        target: target(),
        dest,
        packName: name,
        mcVersion: inst.mc_version,
        loader: inst.loader || null,
        loaderVersion: inst.loader_version || null,
      });
      toast({ type: "success", message: t("components.export.done", { path: out }) });
      props.onClose();
    } catch (e) {
      toast({ type: "error", message: t("components.export.failed", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open={props.open}
      onClose={() => !busy() && props.onClose()}
      label={t("components.export.title")}
      contentClass="w-[420px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden"
    >
      <div class="flex flex-col gap-[16px] p-[20px]">
        <div class="text-[15px] font-bold text-fg">{t("components.export.title")}</div>

        <div class="flex flex-col gap-[8px]">
          <label class="text-[12px] font-semibold text-dim">{t("components.export.formatLabel")}</label>
          <Select value={fmt()} onChange={(v) => setFmt(v as Fmt)} options={formatOptions()} />
          <Show when={fmt() === "modlist"}>
            <Select value={sub()} onChange={setSub} options={subOptions()} />
          </Show>
          <div class="text-[11px] leading-[1.6] text-n-6">{hint()}</div>
        </div>

        <div class="flex justify-end gap-[10px]">
          <button
            disabled={busy()}
            class="h-[34px] px-[16px] border border-glass-border rounded-ctl bg-glass-card text-fg text-[13px] cursor-pointer transition-[background] duration-[var(--dur)] ease-app hover:bg-glass-hover disabled:opacity-50 disabled:cursor-not-allowed"
            onClick={() => props.onClose()}
          >
            {t("components.export.close")}
          </button>
          <button class={ACCENT_BTN} disabled={busy()} onClick={() => void doExport()}>
            {busy() ? t("components.export.exporting") : t("components.export.confirm")}
          </button>
        </div>
      </div>
    </Dialog>
  );
};

export default ExportModpackDialog;
