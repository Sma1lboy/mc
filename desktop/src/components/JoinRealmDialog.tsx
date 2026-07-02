import { useState } from "react";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { useAppStore, activeRoot, refreshInstances, isKobeSignedIn, setCurrentPage } from "../store";
import { t, useLang } from "../i18n";

/**
 * JoinRealmDialog —— 用加入码加入临时领域。加入即建一个绑定该领域的 **pending** 实例
 * (尚未装核心),回调把它的 id 交给调用方(通常直接打开它,在里面点「开始同步」)。
 */
export function JoinRealmDialog(props: {
  open: boolean;
  onClose: () => void;
  onJoined: (instanceId: string) => void;
}) {
  useLang();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const signedIn = useAppStore((s) => s.kobeUser !== null);

  async function submit() {
    if (busy || !code.trim()) return;
    if (!isKobeSignedIn()) {
      toast({ type: "error", message: t("realm.needLogin") });
      return;
    }
    setBusy(true);
    try {
      const instanceId = await api.realmJoin(activeRoot(), code.trim());
      if (!instanceId) {
        toast({ type: "error", message: t("realm.joinBadCode") });
        return;
      }
      refreshInstances();
      toast({ type: "success", message: t("realm.joinedToast") });
      setCode("");
      props.onJoined(instanceId);
    } catch (e) {
      toast({ type: "error", message: t("realm.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={props.open} onClose={props.onClose} label={t("realm.joinTitle")}>
      <div className="p-[20px] flex flex-col gap-[14px]">
        <Heading size="sub" as="h2" className="m-0">
          {t("realm.joinTitle")}
        </Heading>
        {!signedIn && (
          <p className="text-[12px] text-danger-text">
            {t("realm.needLogin")}{" "}
            <button
              className="text-accent hover:underline bg-transparent border-none cursor-pointer p-0"
              onClick={() => {
                props.onClose();
                setCurrentPage("settings");
              }}
            >
              {t("realm.goLogin")}
            </button>
          </p>
        )}
        <label className="flex flex-col gap-[6px]">
          <span className="text-[12px] text-muted">{t("realm.joinCodeLabel")}</span>
          <input
            className="h-[38px] px-[14px] rounded-none text-[13px] text-fg bg-sidebar shadow-input w-full font-mono tracking-[0.16em] uppercase placeholder:text-faint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            type="text"
            maxLength={6}
            placeholder={t("realm.joinCodePlaceholder")}
            value={code}
            onChange={(e) => setCode(e.currentTarget.value.toUpperCase())}
            onKeyDown={(e) => e.key === "Enter" && void submit()}
          />
          <span className="text-[11px] text-faint">{t("realm.joinHint")}</span>
        </label>
        <div className="flex justify-end gap-[8px] mt-[4px]">
          <Button variant="ghost" onClick={props.onClose}>
            {t("realm.cancel")}
          </Button>
          <Button variant="primary" disabled={busy || !code.trim()} onClick={() => void submit()}>
            {busy ? t("realm.joining") : t("realm.joinSubmit")}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

export default JoinRealmDialog;
