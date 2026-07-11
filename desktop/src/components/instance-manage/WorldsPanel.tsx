import { useRef, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Dialog } from "../Dialog";
import { ErrorState } from "../ErrorState";
import { Button } from "../Button";
import { Spinner } from "../Spinner";
import { toast } from "../Toast";
import { api } from "../../ipc/api";
import { openInstanceSubdir } from "../../util/instanceActions";
import { activeRoot } from "../../store";
import { useAsync } from "../../util/useAsync";
import { t } from "../../i18n";
import type { InstanceSummary, WorldInfo } from "../../ipc/types";
import { LABEL, FIELD, DEL_BTN, OPEN_BTN } from "./shared";
import { fmtSize } from "../../util/format";

/**
 * WorldsPanel —— 存档世界列表 + 备份(导出 zip)/ 重命名(改显示名)/ 删除(走回收站)。
 */
export function WorldsPanel(props: { instance: InstanceSummary; tick?: number }) {
  const { data: worlds, loading: worldsLoading, error: worldsError, refetch } = useAsync<WorldInfo[]>(
    () => api.instanceWorlds(activeRoot(), props.instance.id),
    [props.instance.id, props.tick ?? 0],
  );

  // 行内重命名:正在编辑的世界 folder + 草稿名。
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  // 删除存档前确认(存档含游玩进度,删除是破坏性的)。
  const [confirmDel, setConfirmDel] = useState<WorldInfo | null>(null);
  // commitRename 的防重入要读最新 editing(否则 onBlur 的旧闭包会误判为仍在编辑本行)。
  const editingRef = useRef(editing);
  editingRef.current = editing;

  async function importZip() {
    const picked = await openDialog({
      multiple: false,
      filters: [{ name: t("instance.worldsZipFilter"), extensions: ["zip"] }],
      title: t("instance.pickWorldZip"),
    });
    if (typeof picked !== "string") return;
    setImporting(true);
    try {
      const folder = await api.importWorldZip(activeRoot(), props.instance.id, picked);
      toast({ type: "success", message: t("instance.importedWorld", { folder }) });
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.importFailed", { err: String(e) }) });
    } finally {
      setImporting(false);
    }
  }

  async function remove(w: WorldInfo) {
    try {
      await api.deleteWorld(activeRoot(), props.instance.id, w.folder);
      toast({ type: "success", message: t("instance.deletedWorld", { name: w.name }) });
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.deleteFailed", { err: String(e) }) });
    }
  }

  async function backup(w: WorldInfo) {
    // 另存为:用户自定文件名/位置;同名文件由系统对话框确认覆盖,不会静默盖掉上次备份。
    const dest = await saveDialog({
      title: t("instance.backupWorldAs"),
      defaultPath: `${(w.name || w.folder).replace(/[\\/:*?"<>|]/g, "_")}-backup.zip`,
      filters: [{ name: t("instance.zipBackup"), extensions: ["zip"] }],
    });
    if (!dest) return; // 取消
    setBusy(w.folder);
    try {
      const zip = await api.backupWorld(activeRoot(), props.instance.id, w.folder, dest);
      toast({ type: "success", message: t("instance.backedUpTo", { zip }) });
    } catch (e) {
      toast({ type: "error", message: t("instance.backupFailed", { err: String(e) }) });
    } finally {
      setBusy(null);
    }
  }

  function startRename(w: WorldInfo) {
    setDraft(w.name);
    setEditing(w.folder);
  }

  async function commitRename(w: WorldInfo) {
    // 防重入:Enter 提交成功后会 setEditing(null),输入框卸载又触发 onBlur 二次调用;
    // Escape 也先 setEditing(null) 再触发 onBlur。两种情况此时 editing 已不是本行,
    // 直接返回 —— 避免重复重命名/重复 toast,以及「Escape 反而保存」。
    if (editingRef.current !== w.folder) return;
    const name = draft.trim();
    if (!name || name === w.name) {
      setEditing(null);
      return;
    }
    try {
      await api.renameWorld(activeRoot(), props.instance.id, w.folder, name);
      toast({ type: "success", message: t("instance.renamedTo", { name }) });
      setEditing(null);
      refetch();
    } catch (e) {
      toast({ type: "error", message: t("instance.renameFailed", { err: String(e) }) });
    }
  }

  const MODE_LABEL: Record<string, string> = {
    survival: t("instance.modeSurvival"),
    creative: t("instance.modeCreative"),
    adventure: t("instance.modeAdventure"),
    spectator: t("instance.modeSpectator"),
    unknown: t("instance.modeUnknown"),
  };

  return (
    <div className="flex flex-col gap-[8px]">
      <div className="flex items-center justify-between">
        <div className={LABEL}>{t("instance.worldsListTitle")}</div>
        <div className="flex items-center gap-[4px]">
          <button
            className={OPEN_BTN}
            onClick={() => openInstanceSubdir(activeRoot(), props.instance.id, "saves")}
          >
            {t("instance.openDir")}
          </button>
          <button
            className="text-[12px] text-accent px-[8px] py-[3px] rounded-none cursor-pointer hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
            disabled={importing}
            onClick={importZip}
          >
            {importing ? t("instance.importingWorld") : t("instance.importWorld")}
          </button>
        </div>
      </div>

      {worldsLoading ? (
        <div className="flex items-center gap-[10px] text-muted text-[13px] py-[12px]">
          <Spinner size={16} /> {t("instance.scanningWorlds")}
        </div>
      ) : (worlds ?? []).length > 0 ? (
        <div className="flex flex-col gap-[6px]">
          {worlds!.map((w) => (
            <div key={w.folder} className="bg-panel-2 shadow-sunken flex items-center gap-[10px] py-[8px] px-[10px] rounded-none">
              <div className="flex-1 min-w-0">
                {editing === w.folder ? (
                  <input
                    className={`${FIELD} h-[26px] w-full text-[12px]`}
                    ref={(el) => {
                      if (el) queueMicrotask(() => el.focus());
                    }}
                    name="worldName"
                    autoComplete="off"
                    spellCheck={false}
                    value={draft}
                    onChange={(e) => setDraft(e.currentTarget.value)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename(w);
                      else if (e.key === "Escape") setEditing(null);
                    }}
                    onBlur={() => commitRename(w)}
                  />
                ) : (
                  <div className="text-[13px] text-fg whitespace-nowrap overflow-hidden text-ellipsis">{w.name}</div>
                )}
                <div className="text-[11px] text-muted whitespace-nowrap overflow-hidden text-ellipsis">
                  {[
                    MODE_LABEL[w.game_mode] ?? w.game_mode,
                    fmtSize(w.size_bytes),
                    w.seed != null ? t("instance.seed", { seed: w.seed }) : null,
                    w.folder,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </div>
              </div>
              <button
                className="shrink-0 text-[12px] text-muted px-[8px] py-[4px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2 disabled:opacity-50 disabled:cursor-default"
                disabled={busy === w.folder}
                onClick={() => backup(w)}
              >
                {busy === w.folder ? t("instance.backingUp") : t("instance.backup")}
              </button>
              <button
                className="shrink-0 text-[12px] text-muted px-[8px] py-[4px] rounded-none cursor-pointer hover:text-fg hover:bg-panel-2"
                onClick={() => startRename(w)}
              >
                {t("instance.rename")}
              </button>
              <button className={DEL_BTN} onClick={() => setConfirmDel(w)}>
                {t("instance.delete")}
              </button>
            </div>
          ))}
        </div>
      ) : worldsError ? (
        <ErrorState compact message={t("instance.worldsLoadError")} onRetry={() => void refetch()} />
      ) : (
        <div className="text-muted text-[13px] py-[12px]">{t("instance.noWorlds")}</div>
      )}

      <Dialog
        open={confirmDel !== null}
        onClose={() => setConfirmDel(null)}
        label={t("instance.deleteWorld")}
        contentClass="w-[360px] max-w-[calc(100vw-48px)] overflow-hidden"
      >
        <div className="p-[20px] flex flex-col gap-[14px]">
          <div className="text-[15px] font-semibold text-fg">
            {t("instance.deleteWorldConfirm", { name: confirmDel?.name ?? "" })}
          </div>
          <div className="text-[13px] text-muted leading-[1.6]">{t("instance.deleteWorldBody")}</div>
          <div className="flex justify-end gap-[10px]">
            <Button variant="ghost" onClick={() => setConfirmDel(null)}>
              {t("instance.cancel")}
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                const w = confirmDel;
                setConfirmDel(null);
                if (w) void remove(w);
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
