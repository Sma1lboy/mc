// 实例操作 —— Home / Library 等页面共享的实例上下文菜单动作实现。
// InstanceRow 只发出 onPlay/onOpenDir/onExport/onDelete 事件,真正的副作用
// (打开目录、导出、删除)集中在这里,避免在每个页面重复一遍。
import { api } from "../ipc/api";
import { isRunning } from "../store";
import { toast } from "../components";
import { t } from "../i18n";

/** 打开实例的游戏目录(向后端取路径后用系统文件管理器打开)。 */
export async function openInstanceDir(root: string, id: string): Promise<void> {
  try {
    const dir = await api.instanceDir(root, id);
    await api.revealPath(dir);
  } catch (e) {
    toast({ type: "error", message: t("store.instance.openDirFailed", { error: String(e) }) });
  }
}

/** 打开实例的某个子目录(mods/resourcepacks/saves/screenshots…),后端确保目录存在。 */
export async function openInstanceSubdir(root: string, id: string, sub: string): Promise<void> {
  try {
    const dir = await api.instanceSubdir(root, id, sub);
    await api.revealPath(dir);
  } catch (e) {
    toast({ type: "error", message: t("store.instance.openDirFailed", { error: String(e) }) });
  }
}

/** 删除实例(调用方应已确认)。成功返回 true,供页面刷新列表。 */
export async function deleteInstance(
  root: string,
  inst: { id: string; name: string },
): Promise<boolean> {
  // 运行中的实例不能删:游戏正占着目录里的文件(Windows 会锁定、各平台都可能删半截)。
  if (isRunning(inst.id)) {
    toast({ type: "error", message: t("store.instance.stopBeforeDelete") });
    return false;
  }
  try {
    await api.deleteInstance(root, inst.id);
    toast({ type: "success", message: t("store.instance.deleted", { name: inst.name }) });
    return true;
  } catch (e) {
    toast({ type: "error", message: t("store.instance.deleteFailed", { error: String(e) }) });
    return false;
  }
}
