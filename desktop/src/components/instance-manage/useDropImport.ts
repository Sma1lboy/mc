import { useEffect, useRef, useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { toast } from "../Toast";
import { api } from "../../ipc/api";
import { activeRoot } from "../../store";
import { t } from "../../i18n";
import type { InstanceSummary } from "../../ipc/types";
import { isPackTab, type InstanceManageTab } from "./shared";

/**
 * useDropImport —— 管理页的拖拽导入(mods / 资源包 / 光影 / 数据包 / 存档)。
 * 原样收拢自 InstanceManageDialog 内联逻辑:监听只随 active 重订阅,tab/instance
 * 走 ref 取实时值;导入成功后按标签回调刷新(mods 重拉,其余走 tick)。
 */
export function useDropImport(args: {
  instance: InstanceSummary | null;
  active: boolean;
  tab: InstanceManageTab;
  /** mods 导入成功后重拉列表。 */
  onModsImported: () => void;
}) {
  const { instance, active, tab } = args;
  // Tauri 启用了原生文件拖放,HTML5 ondrop 不触发,改用 webview 的 onDragDropEvent。
  const [dragOver, setDragOver] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [importTick, setImportTick] = useState(0);
  const [worldTick, setWorldTick] = useState(0);

  // 拖放事件在监听装好之后才触发,需读「当时」的 tab/instance/active —— 走 ref 取实时值,
  // 让监听只随 active 重订阅(镜像 Solid effect 只追踪 active())。
  const tabRef = useRef(tab);
  tabRef.current = tab;
  const activeRef = useRef(active);
  activeRef.current = active;
  const instanceRef = useRef(instance);
  instanceRef.current = instance;

  /** 当前标签接受拖拽吗(设置标签不接受)。 */
  function dropAcceptedFor(cur: InstanceManageTab): boolean {
    return cur === "mods" || isPackTab(cur) || cur === "worlds";
  }

  /** mods/资源包/光影/数据包的导入目标类型(存档走单独的 zip 导入命令,这里返回 null)。 */
  function resourceTargetFor(cur: InstanceManageTab): string | null {
    if (cur === "mods") return "mod";
    if (isPackTab(cur)) return cur === "resource_pack" ? "resourcepack" : cur;
    return null;
  }

  async function handleDrop(paths: string[]) {
    const inst = instanceRef.current;
    const cur = tabRef.current;
    if (!inst || !dropAcceptedFor(cur)) {
      toast({ type: "info", message: t("instance.dropHint") });
      return;
    }
    setDropping(true);
    try {
      // 并行导入(串行会让拖入多个大文件逐个卡住);用 allSettled 汇总成败。
      const results = await Promise.allSettled(
        paths.map((path) =>
          cur === "worlds"
            ? api.importWorldZip(activeRoot(), inst.id, path)
            : api.importLocalResource(activeRoot(), inst.id, resourceTargetFor(cur)!, path, null),
        ),
      );
      const ok = results.filter((r) => r.status === "fulfilled").length;
      const failed = results.length - ok;
      if (ok > 0) {
        if (cur === "mods") args.onModsImported();
        else if (cur === "worlds") setWorldTick((x) => x + 1);
        else setImportTick((x) => x + 1);
      }
      // 单条汇总,而不是每个失败弹一条 + 末尾静默。
      if (failed === 0) toast({ type: "success", message: t("instance.importedFiles", { n: ok }) });
      else if (ok === 0) toast({ type: "error", message: t("instance.importFilesFailed", { n: failed }) });
      else toast({ type: "warn", message: t("instance.importFilesPartial", { ok, failed }) });
    } finally {
      setDropping(false);
    }
  }

  useEffect(() => {
    if (!active) return;
    const unlisten = getCurrentWebview().onDragDropEvent((e) => {
      if (!activeRef.current) return;
      const p = e.payload;
      if (p.type === "enter" || p.type === "over") setDragOver(true);
      else if (p.type === "leave") setDragOver(false);
      else if (p.type === "drop") {
        setDragOver(false);
        void handleDrop(p.paths);
      }
    });
    return () => void unlisten.then((f) => f());
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active]);

  return { dragOver, dropping, importTick, worldTick, dropAccepted: dropAcceptedFor(tab) };
}
