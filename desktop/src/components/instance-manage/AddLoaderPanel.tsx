import { useEffect, useRef, useState } from "react";
import { Spinner } from "../Spinner";
import { Select } from "../Select";
import { toast } from "../Toast";
import { api, onInstallProgress } from "../../ipc/api";
import { activeRoot } from "../../store";
import { t } from "../../i18n";
import type { InstanceSummary } from "../../ipc/types";
import { FIELD } from "./shared";

/** 加载器选项(与后端 parse_loader_kind 对齐)。 */
const LOADER_OPTS = [
  { label: "Fabric", value: "fabric" },
  { label: "Quilt", value: "quilt" },
  { label: "Forge", value: "forge" },
  { label: "NeoForge", value: "neoforge" },
];

/**
 * AddLoaderPanel —— 原版实例「加装核心」:选加载器(+ Forge/NeoForge 版本)→ install_loader。
 * 装完后端可能换实例 id(实例目录名恰为原版号的退化情形),回调把新 id 传出去重定向。
 */
export function AddLoaderPanel(props: { instance: InstanceSummary; onAdded: (newId: string) => void }) {
  const [loader, setLoader] = useState("fabric");
  const [version, setVersion] = useState("");
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState("");
  const needsVersion = loader === "forge" || loader === "neoforge";
  // 进度回调装一次即可,却要读最新 busy —— 用 ref 避免旧闭包永远看到 busy=false。
  const busyRef = useRef(busy);
  busyRef.current = busy;

  useEffect(
    () =>
      onInstallProgress((p) => {
        if (!busyRef.current) return;
        setProgress(p.total > 0 ? `${p.stage} ${p.current}/${p.total}` : p.stage);
      }),
    [],
  );

  async function add() {
    if (busy) return;
    if (needsVersion && !version.trim()) {
      toast({ type: "error", message: t("instance.fillForgeVersion") });
      return;
    }
    setBusy(true);
    setProgress(t("instance.preparing"));
    try {
      const newId = await api.installLoader(
        activeRoot(),
        props.instance.id,
        props.instance.mc_version,
        loader,
        needsVersion ? version.trim() : null,
      );
      toast({ type: "success", message: t("instance.loaderAdded") });
      props.onAdded(newId);
    } catch (e) {
      toast({ type: "error", message: t("instance.addLoaderFailed", { err: String(e) }) });
    } finally {
      setBusy(false);
      setProgress("");
    }
  }

  return (
    <div className="flex flex-col gap-[10px] py-[4px]">
      <div className="text-[13px] text-muted leading-[1.6]">{t("instance.addLoaderIntro")}</div>
      <div className="flex items-center gap-[8px]">
        <Select value={loader} onChange={setLoader} options={LOADER_OPTS} />
        {needsVersion && (
          <input
            className={`${FIELD} flex-1`}
            placeholder={loader === "forge" ? t("instance.forgeBuildPlaceholder") : t("instance.neoforgeVersionPlaceholder")}
            value={version}
            onChange={(e) => setVersion(e.currentTarget.value)}
          />
        )}
        <button
          className="shrink-0 h-[34px] px-[14px] rounded-none bg-accent text-white shadow-raised text-[13px] font-semibold cursor-pointer transition-[box-shadow,background-color] duration-[var(--dur)] ease-app hover:bg-accent-hover active:shadow-pressed disabled:opacity-50 disabled:cursor-default"
          disabled={busy}
          onClick={add}
        >
          {busy ? t("instance.installing") : t("instance.addLoader")}
        </button>
      </div>
      {busy && progress && (
        <div className="flex items-center gap-[8px] text-accent text-[12px]">
          <Spinner size={14} /> {progress}
        </div>
      )}
    </div>
  );
}
