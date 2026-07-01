import { useState } from "react";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { useAppStore, refreshIdentities } from "../store";
import { useAsync } from "../util/useAsync";
import { t, useLang } from "../i18n";
import type { AccountSummary, Identity } from "../ipc/bindings";

/**
 * LinkedAccountsSection —— kobeMC 账号下拉里的「关联账号」区:把本地已验证的微软身份
 * (账号的 MC 资料 UUID)绑定到当前 kobeMC 用户,展示已关联的身份并支持解绑。
 * 仅在登录 kobeMC 后由 KobeAccountChip 渲染。
 */
export function LinkedAccountsSection(): React.ReactElement {
  useLang();
  // 已关联身份来自 store(单一真相);listAccounts 是本地廉价调用,保留本地 resource。
  const identities = useAppStore((s) => s.accountIdentities);
  const { data: accounts } = useAsync(() => api.listAccounts(), []);
  const [busy, setBusy] = useState(false);

  const msAccounts = (accounts ?? []).filter((a) => a.kind === "microsoft");
  const isLinked = (acc: AccountSummary) =>
    (identities ?? []).some((i) => i.provider === "microsoft" && i.account_id === acc.uuid);

  const providerLabel = (i: Identity) => {
    if (i.provider === "microsoft") return t("link.providerMicrosoft");
    if (i.provider === "credential") return t("link.providerCredential");
    return i.provider;
  };

  async function act(fn: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      toast({ type: "error", message: t("link.opError", { err: String(e) }) });
    } finally {
      setBusy(false);
    }
  }

  const link = (acc: AccountSummary) =>
    act(async () => {
      await api.accountLinkMicrosoft(acc.uuid, acc.username);
      toast({ type: "success", message: t("link.linked") });
      void refreshIdentities();
    });

  const unlink = (provider: string) =>
    act(async () => {
      await api.accountUnlink(provider);
      toast({ type: "success", message: t("link.unlinked") });
      void refreshIdentities();
    });

  return (
    <div className="mt-[12px] pt-[12px] border-t border-titlebar">
      <div className="text-[13px] text-strong font-display mb-[6px]">{t("link.title")}</div>
      <p className="text-[12px] text-muted leading-[1.5] mb-[8px]">{t("link.hint")}</p>

      {/* 已关联的身份 */}
      {(identities ?? []).length > 0 && (
        <div className="flex flex-col gap-[4px] mb-[8px]">
          {(identities ?? []).map((i) => (
            <div key={i.provider} className="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
              <span className="text-[13px] text-fg truncate flex-1">{providerLabel(i)}</span>
              <span className="text-[11px] text-faint truncate max-w-[80px]" title={i.account_id}>
                {i.account_id.slice(0, 8)}
              </span>
              {i.provider !== "credential" && (
                <button
                  className="text-[11px] text-danger-text hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                  disabled={busy}
                  onClick={() => void unlink(i.provider)}
                >
                  {t("link.unlink")}
                </button>
              )}
            </div>
          ))}
        </div>
      )}

      {/* 绑定微软账号 */}
      {msAccounts.length > 0 ? (
        <div className="flex flex-col gap-[4px]">
          {msAccounts.map((acc) => (
            <div key={acc.uuid} className="flex items-center gap-[8px] px-[2px] py-[3px]">
              <span className="text-[13px] text-fg truncate flex-1">{acc.username}</span>
              {!isLinked(acc) ? (
                <button
                  className="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                  disabled={busy}
                  onClick={() => void link(acc)}
                >
                  {t("link.bind")}
                </button>
              ) : (
                <span className="text-[11px] text-muted">{t("link.alreadyLinked")}</span>
              )}
            </div>
          ))}
        </div>
      ) : (
        <p className="text-[12px] text-faint leading-[1.5]">{t("link.noMsAccount")}</p>
      )}
    </div>
  );
}
