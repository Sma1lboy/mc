import { Component, createResource, createSignal, For, Show } from "solid-js";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { t } from "../i18n";
import type { AccountSummary, Identity } from "../ipc/bindings";

/**
 * LinkedAccountsSection —— kobeMC 账号下拉里的「关联账号」区:把本地已验证的微软身份
 * (账号的 MC 资料 UUID)绑定到当前 kobeMC 用户,展示已关联的身份并支持解绑。
 * 仅在登录 kobeMC 后由 KobeAccountChip 渲染。
 */
export const LinkedAccountsSection: Component = () => {
  const [identities, { refetch }] = createResource(() => api.accountIdentities());
  const [accounts] = createResource(() => api.listAccounts());
  const [busy, setBusy] = createSignal(false);

  const msAccounts = () => (accounts() ?? []).filter((a) => a.kind === "microsoft");
  const isLinked = (acc: AccountSummary) =>
    (identities() ?? []).some((i) => i.provider === "microsoft" && i.account_id === acc.uuid);

  const providerLabel = (i: Identity) => {
    if (i.provider === "microsoft") return t("link.providerMicrosoft");
    if (i.provider === "credential") return t("link.providerCredential");
    return i.provider;
  };

  async function act(fn: () => Promise<void>) {
    if (busy()) return;
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
      void refetch();
    });

  const unlink = (provider: string) =>
    act(async () => {
      await api.accountUnlink(provider);
      toast({ type: "success", message: t("link.unlinked") });
      void refetch();
    });

  return (
    <div class="mt-[12px] pt-[12px] border-t border-titlebar">
      <div class="text-[13px] text-strong font-display mb-[6px]">{t("link.title")}</div>
      <p class="text-[12px] text-muted leading-[1.5] mb-[8px]">{t("link.hint")}</p>

      {/* 已关联的身份 */}
      <Show when={(identities() ?? []).length > 0}>
        <div class="flex flex-col gap-[4px] mb-[8px]">
          <For each={identities()}>
            {(i) => (
              <div class="flex items-center gap-[8px] bg-sidebar shadow-input px-[8px] py-[5px]">
                <span class="text-[13px] text-fg truncate flex-1">{providerLabel(i)}</span>
                <span class="text-[11px] text-faint truncate max-w-[80px]" title={i.account_id}>
                  {i.account_id.slice(0, 8)}
                </span>
                <Show when={i.provider !== "credential"}>
                  <button
                    class="text-[11px] text-danger-text hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                    disabled={busy()}
                    onClick={() => void unlink(i.provider)}
                  >
                    {t("link.unlink")}
                  </button>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* 绑定微软账号 */}
      <Show
        when={msAccounts().length > 0}
        fallback={<p class="text-[12px] text-faint leading-[1.5]">{t("link.noMsAccount")}</p>}
      >
        <div class="flex flex-col gap-[4px]">
          <For each={msAccounts()}>
            {(acc) => (
              <div class="flex items-center gap-[8px] px-[2px] py-[3px]">
                <span class="text-[13px] text-fg truncate flex-1">{acc.username}</span>
                <Show
                  when={!isLinked(acc)}
                  fallback={<span class="text-[11px] text-muted">{t("link.alreadyLinked")}</span>}
                >
                  <button
                    class="text-[12px] text-accent hover:underline bg-transparent border-none cursor-pointer disabled:opacity-50"
                    disabled={busy()}
                    onClick={() => void link(acc)}
                  >
                    {t("link.bind")}
                  </button>
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </div>
  );
};
