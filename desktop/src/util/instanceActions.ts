// 实例操作 —— Home / Library 等页面共享的实例上下文菜单动作实现。
// InstanceRow 只发出 onPlay/onOpenDir/onExport/onDelete 事件,真正的副作用
// (打开目录、导出、删除)集中在这里,避免在每个页面重复一遍。
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../ipc/api";
import { isRunning } from "../store";
import { toast, type InstanceRowData } from "../components";
import type { ImportOutcome } from "../ipc/types";

/**
 * 选本地 `.mrpack`/`.zip` 导入为实例。两套布局共用(经典启动页 + 工作台库)。
 * 返回成功导入的产物(含 blocked / skipped,调用方据此弹手动下载对话框);取消返回 null。
 * 失败已在此 toast,调用方拿到 null 即可。
 */
export async function importModpackFile(root: string): Promise<ImportOutcome | null> {
  const picked = await openDialog({
    title: "选择整合包",
    multiple: false,
    filters: [{ name: "整合包", extensions: ["mrpack", "zip"] }],
  }).catch(() => null);
  if (!picked || typeof picked !== "string") return null;
  try {
    return await api.importModpack(root, picked, null);
  } catch (e) {
    toast({ type: "error", message: `导入失败:${e}` });
    return null;
  }
}

/** 打开实例的游戏目录(向后端取路径后用系统文件管理器打开)。 */
export async function openInstanceDir(root: string, id: string): Promise<void> {
  try {
    const dir = await api.instanceDir(root, id);
    await api.revealPath(dir);
  } catch (e) {
    toast({ type: "error", message: `打开目录失败:${e}` });
  }
}

/** 打开实例的某个子目录(mods/resourcepacks/saves/screenshots…),后端确保目录存在。 */
export async function openInstanceSubdir(root: string, id: string, sub: string): Promise<void> {
  try {
    const dir = await api.instanceSubdir(root, id, sub);
    await api.revealPath(dir);
  } catch (e) {
    toast({ type: "error", message: `打开目录失败:${e}` });
  }
}

/** 删除实例(调用方应已确认)。成功返回 true,供页面刷新列表。 */
export async function deleteInstance(
  root: string,
  inst: { id: string; name: string },
): Promise<boolean> {
  // 运行中的实例不能删:游戏正占着目录里的文件(Windows 会锁定、各平台都可能删半截)。
  if (isRunning(inst.id)) {
    toast({ type: "error", message: "请先停止运行中的游戏,再删除该实例" });
    return false;
  }
  try {
    await api.deleteInstance(root, inst.id);
    toast({ type: "success", message: `已删除实例「${inst.name}」` });
    return true;
  } catch (e) {
    toast({ type: "error", message: `删除失败:${e}` });
    return false;
  }
}

/** 导出实例为 .mrpack(弹保存对话框选目标路径,走 Modrinth 目标)。 */
export async function exportInstanceMrpack(
  root: string,
  row: InstanceRowData,
): Promise<void> {
  try {
    const dest = await saveDialog({
      title: "导出整合包",
      defaultPath: `${row.name}.mrpack`,
      filters: [{ name: "Modrinth 整合包", extensions: ["mrpack"] }],
    });
    if (!dest) return; // 用户取消
    toast({ type: "info", message: "正在导出整合包…" });
    const out = await api.exportModpack({
      root,
      instanceId: row.id,
      target: "modrinth",
      dest,
      packName: row.name,
      mcVersion: row.mc_version,
      loader: row.loader || null,
      loaderVersion: row.loader_version || null,
    });
    toast({ type: "success", message: `已导出:${out}` });
  } catch (e) {
    toast({ type: "error", message: `导出失败:${e}` });
  }
}
