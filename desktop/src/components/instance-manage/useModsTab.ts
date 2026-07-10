import { useState } from "react";
import { toast } from "../Toast";
import { api } from "../../ipc/api";
import { activeRoot } from "../../store";
import { useAsync } from "../../util/useAsync";
import { t } from "../../i18n";
import type { ContentProvider } from "../ContentBrowser";
import type { ModpackHit } from "../ModpackCard";
import type { InstanceSummary, ModInfo, ModUpdate } from "../../ipc/types";

/**
 * useModsTab —— Mods 标签的全部状态与动作(列表 / 搜索安装 / 更新检查 / 启停删除)。
 * 只是把原先内联在 InstanceManageDialog 里的这段逻辑收进一个 hook:仍在父组件的
 * 渲染中运行,挂载与状态生命周期与内联时完全一致。gate 未满足时不真正打后端。
 */
export function useModsTab(instance: InstanceSummary | null, gated: boolean) {
  const { data: mods, loading: modsLoading, error: modsError, refetch: refetchMods } = useAsync<ModInfo[] | undefined>(
    () => (gated && instance ? api.instanceMods(activeRoot(), instance.id) : Promise.resolve(undefined)),
    [gated, instance?.id],
  );

  // ---- 从 Modrinth 搜索并安装 ----
  // vanilla 实例没有加载器,搜 mod 无意义,这里把 loader 归一为 null(不限)。
  const searchLoader = (() => {
    const l = instance?.loader;
    return l && l !== "vanilla" ? l : null;
  })();
  const [modDetail, setModDetail] = useState<ModpackHit | null>(null);
  const [modDetailProvider, setModDetailProvider] = useState<ContentProvider>("modrinth");
  // 后台并行安装:正在安装的 project_id 集合(不阻塞其它行)。
  const [installing, setInstalling] = useState<Set<string>>(new Set());
  // 本次浏览已添加的 mod project_id:行按钮即时变「已添加」。
  const [addedMods, setAddedMods] = useState<Set<string>>(new Set());
  // 删除 mod 前确认(删除是破坏性的,与存档/资源包删除一致)。
  const [confirmDelMod, setConfirmDelMod] = useState<ModInfo | null>(null);
  // 行内「下载」:直接装最新兼容版(解析依赖),不进详情;后台并行,不阻塞其它行。
  async function installHit(projectId: string, title: string, provider: ContentProvider = "modrinth") {
    const inst = instance;
    if (!inst || installing.has(projectId)) return;
    setInstalling((s) => new Set(s).add(projectId));
    try {
      const report = await api.installMod(activeRoot(), inst.id, projectId, inst.mc_version, searchLoader ?? "", provider);
      if ((report.blocked?.length ?? 0) > 0) {
        toast({ type: "warn", message: t("instance.blockedManual", { count: report.blocked!.length }) });
      } else {
        if (report.installed.length === 0 && report.unresolved.length === 0) {
          toast({ type: "info", message: t("instance.modExists", { title }) });
        } else {
          const parts = [t("instance.modInstalledCount", { n: report.installed.length })];
          if (report.unresolved.length > 0) parts.push(t("instance.modUnresolvedCount", { n: report.unresolved.length }));
          toast({
            type: report.unresolved.length > 0 ? "warn" : "success",
            message: t("instance.modInstallResult", { title, parts: parts.join(",") }),
          });
        }
        setAddedMods((s) => new Set(s).add(projectId));
        refetchMods();
      }
    } catch (e) {
      toast({ type: "error", message: t("instance.installFailed", { err: String(e) }) });
    } finally {
      setInstalling((s) => {
        const n = new Set(s);
        n.delete(projectId);
        return n;
      });
    }
  }

  // ---- Mod 更新检查 ----
  const [updates, setUpdates] = useState<ModUpdate[] | null>(null);
  const [checking, setChecking] = useState(false);
  // 后台并行更新:正在更新的文件集合(不阻塞其它行/全部更新串行)。
  const [updating, setUpdating] = useState<Set<string>>(new Set());

  async function checkUpdates() {
    const inst = instance;
    if (!inst) return;
    setChecking(true);
    try {
      const list = await api.checkModUpdates(activeRoot(), inst.id, inst.mc_version, searchLoader ?? "");
      setUpdates(list);
      toast({
        type: list.length > 0 ? "info" : "success",
        message: list.length > 0 ? t("instance.foundUpdates", { n: list.length }) : t("instance.allModsUpToDate"),
      });
    } catch (e) {
      toast({ type: "error", message: t("instance.checkUpdatesFailed", { err: String(e) }) });
    } finally {
      setChecking(false);
    }
  }

  async function applyUpdate(u: ModUpdate) {
    const inst = instance;
    if (!inst || updating.has(u.file_name)) return;
    setUpdating((s) => new Set(s).add(u.file_name));
    try {
      await api.applyModUpdate(activeRoot(), inst.id, u);
      toast({ type: "success", message: t("instance.modUpdated", { name: u.name, version: u.new_version }) });
      setUpdates((prev) => (prev ?? []).filter((x) => x.file_name !== u.file_name));
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: t("instance.updateFailed", { err: String(e) }) });
    } finally {
      setUpdating((s) => {
        const n = new Set(s);
        n.delete(u.file_name);
        return n;
      });
    }
  }

  async function applyAllUpdates() {
    // 后台并行更新,不阻塞单项;每个失败只提示该项,不中断其余。
    await Promise.all((updates ?? []).map((u) => applyUpdate(u)));
  }

  async function toggleMod(m: ModInfo, enabled: boolean) {
    const inst = instance;
    if (!inst) return;
    try {
      await api.setModEnabled(activeRoot(), inst.id, m.file_name, enabled);
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: t("instance.opFailed", { err: String(e) }) });
    }
  }

  async function removeMod(m: ModInfo) {
    const inst = instance;
    if (!inst) return;
    try {
      await api.deleteMod(activeRoot(), inst.id, m.file_name);
      toast({ type: "success", message: t("instance.deletedMod", { name: m.name }) });
      refetchMods();
    } catch (e) {
      toast({ type: "error", message: t("instance.deleteModFailed", { err: String(e) }) });
    }
  }

  return {
    mods, modsLoading, modsError, refetchMods,
    searchLoader,
    modDetail, setModDetail, modDetailProvider, setModDetailProvider,
    installing, addedMods, setAddedMods,
    confirmDelMod, setConfirmDelMod,
    installHit,
    updates, setUpdates, checking, updating, checkUpdates, applyUpdate, applyAllUpdates,
    toggleMod, removeMod,
  };
}

export type ModsTabState = ReturnType<typeof useModsTab>;
