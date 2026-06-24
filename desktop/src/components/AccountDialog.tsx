import { Component, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner } from "./Spinner";
import { ErrorState } from "./ErrorState";
import { Dialog } from "./Dialog";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { accountKindLabel } from "../util/accounts";
import { t } from "../i18n";
import type { AccountSummary, DeviceCode } from "../ipc/types";


// 登录表单按钮:抽公共件,确保离线/外置两套表单的取消/提交按钮完全一致(过渡也不漏)。
const ACCOUNT_CANCEL_BTN =
  "h-[36px] px-[18px] border border-glass-border rounded-xs bg-glass-card text-fg text-[13px] " +
  "cursor-pointer transition-[background-color,border-color] duration-[var(--dur)] ease-app hover:bg-glass-hover hover:border-a-4";
const ACCOUNT_SUBMIT_BTN =
  "h-[36px] px-[18px] rounded-xs bg-a-5 text-white border border-a-5 text-[13px] cursor-pointer " +
  "transition-opacity duration-[var(--dur)] ease-app hover:not-disabled:opacity-90 disabled:opacity-50 disabled:cursor-not-allowed";
// 账号表单输入框(离线用户名 + 外置登录三项)统一样式,避免逐个内联漂移。
const ACCOUNT_INPUT =
  "h-[38px] px-[14px] border border-glass-border rounded-xs text-[13px] text-fg glass-input " +
  "transition-[border-color,background-color,box-shadow] duration-150 ease-app " +
  "focus-visible:outline-none focus-visible:border-a-4 focus-visible:bg-glass-card focus-visible:ring-2 focus-visible:ring-a-4/25";

/** 登录弹窗状态机:选择方式 → 微软设备码 / 离线用户名 / 外置登录。 */
type Step = "menu" | "msa" | "offline" | "yggdrasil";

/**
 * AccountDialog —— 主题中性的账号登录弹窗(微软正版 + 离线),两套布局共用。
 *
 * 用桥接令牌(fg / dim / card / 中性 n- / 强调 a-)着色,故在工作台(深色)与经典(浅色)两种布局下
 * 都自动对味。微软走设备码流:start 拿 user_code + 验证地址 → 自动打开浏览器并复制代码 →
 * 后台 poll 阻塞直到用户在网页完成 → 落库并选中 → 回调 onDone。
 */
export const AccountDialog: Component<{
  onClose: () => void;
  onDone: (acc: AccountSummary) => void;
}> = (props) => {
  const [step, setStep] = createSignal<Step>("menu");
  const [device, setDevice] = createSignal<DeviceCode | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [offlineName, setOfflineName] = createSignal("");
  const [ygBase, setYgBase] = createSignal("");
  const [ygUser, setYgUser] = createSignal("");
  const [ygPass, setYgPass] = createSignal("");

  // 已有账号:菜单步顶部列出,可切换/删除(两套布局共用的账号管理入口)。
  const [accounts, { refetch: refetchAccounts }] = createResource(() => api.listAccounts());
  const accountList = () => accounts() ?? [];
  // 正在切换/移除的账号 uuid:列表常驻可见,异步期间禁用该行防重复触发。
  const [pendingAcc, setPendingAcc] = createSignal<string | null>(null);

  async function selectExisting(acc: AccountSummary) {
    if (acc.selected) {
      props.onDone(acc);
      return;
    }
    if (pendingAcc()) return;
    setPendingAcc(acc.uuid);
    try {
      await api.selectAccount(acc.uuid);
      props.onDone(acc);
    } catch (e) {
      setError(String(e));
    } finally {
      if (!closed) setPendingAcc(null);
    }
  }

  async function removeExisting(acc: AccountSummary, e: MouseEvent) {
    e.stopPropagation();
    if (pendingAcc()) return;
    setPendingAcc(acc.uuid);
    try {
      await api.removeAccount(acc.uuid);
      toast({ type: "success", message: t("account.removed", { name: acc.username }) });
      void refetchAccounts();
    } catch (err) {
      toast({ type: "error", message: t("account.removeFailed", { err: String(err) }) });
    } finally {
      if (!closed) setPendingAcc(null);
    }
  }

  // 弹窗关闭后别再回调(微软 poll 可能仍在后台轮询)。
  let closed = false;
  onCleanup(() => {
    closed = true;
  });

  async function startMsa() {
    setStep("msa");
    setError(null);
    setBusy(true);
    try {
      const info = await api.msaLoginStart();
      setDevice(info);
      // 自动复制代码 + 打开微软验证页,省去用户手抄。
      try {
        await navigator.clipboard.writeText(info.user_code);
      } catch {
        /* 剪贴板不可用时忽略,代码已在弹窗里大字显示 */
      }
      try {
        await shellOpen(info.verification_uri);
      } catch {
        /* 打不开浏览器也没关系,地址已显示,用户可手动访问 */
      }
      // 阻塞轮询直到用户完成(后端内部按 interval 轮询)。
      const acc = await api.msaLoginPoll(info.device_code, info.interval);
      if (closed) return;
      toast({ type: "success", message: t("account.loggedIn", { name: acc.username }) });
      props.onDone(acc);
    } catch (e) {
      if (closed) return;
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function submitOffline(e: Event) {
    e.preventDefault();
    const name = offlineName().trim();
    if (!name) return;
    setBusy(true);
    setError(null);
    try {
      const acc = await api.addOfflineAccount(name);
      if (closed) return;
      toast({ type: "success", message: t("account.offlineAdded", { name: acc.username }) });
      props.onDone(acc);
    } catch (e) {
      if (closed) return;
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function submitYggdrasil(e: Event) {
    e.preventDefault();
    const base = ygBase().trim();
    const user = ygUser().trim();
    if (!base || !user) return;
    setBusy(true);
    setError(null);
    try {
      const acc = await api.yggdrasilLogin(base, user, ygPass());
      if (closed) return;
      toast({ type: "success", message: t("account.loggedInYggdrasil", { name: acc.username }) });
      props.onDone(acc);
    } catch (e) {
      if (closed) return;
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const title = () =>
    step() === "offline"
      ? t("account.titleOffline")
      : step() === "msa"
        ? t("account.titleMsa")
        : step() === "yggdrasil"
          ? t("account.titleYggdrasil")
          : t("account.titleAdd");

  return (
    <Dialog
      open
      onClose={props.onClose}
      label={title()}
      contentClass="w-[380px] max-w-[calc(100vw-48px)] glass-pop rounded-card overflow-hidden focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-4"
    >
      <div class="flex items-center justify-between px-[18px] py-[14px] text-[15px] font-bold text-white bg-[linear-gradient(90deg,var(--a-6),var(--a-5))]">
        <span>{title()}</span>
        <button
          class="border-none bg-transparent text-white cursor-pointer opacity-85 p-[5px] rounded-xs flex items-center hover:bg-white/20 hover:opacity-100"
          onClick={props.onClose}
          aria-label={t("account.close")}
        >
          <Icon name="close" size={16} />
        </button>
      </div>

      {/* --- 选择登录方式 --- */}
      <Show when={step() === "menu"}>
        <div class="p-[18px] flex flex-col gap-[12px]">
          {/* 账号列表加载失败:给错误态 + 重试,别让失败看起来像「没有账号」。 */}
          <Show when={accounts.error}>
            <ErrorState compact message={t("account.listLoadFailed")} onRetry={() => void refetchAccounts()} />
          </Show>
          {/* 已有账号:切换(点击)或移除(✕)。当前账号打勾。 */}
          <Show when={accountList().length > 0}>
            <div class="flex flex-col gap-[6px]">
              <For each={accountList()}>
                {(acc) => (
                  <div
                    role="button"
                    tabindex={0}
                    aria-busy={pendingAcc() === acc.uuid}
                    class="group flex items-center gap-[10px] px-[10px] py-[8px] rounded-ctl glass-card cursor-pointer transition-[background-color,border-color] duration-150 hover:bg-a-1 hover:border-a-4 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5 aria-[busy=true]:opacity-60 aria-[busy=true]:pointer-events-none"
                    classList={{ "!border-a-4 !bg-a-1": acc.selected }}
                    onClick={() => selectExisting(acc)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" || e.key === " ") {
                        e.preventDefault();
                        void selectExisting(acc);
                      }
                    }}
                  >
                    <span class="w-[30px] h-[30px] flex-[0_0_30px] rounded-xs grid place-items-center text-white text-[13px] font-semibold bg-[linear-gradient(135deg,var(--a-3),var(--a-5))]">
                      <Avatar kind={acc.kind} uuid={acc.uuid} />
                    </span>
                    <span class="min-w-0 flex-1">
                      <span class="block text-[13px] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                        {acc.username}
                      </span>
                      <span class="block text-[11px] text-dim">{accountKindLabel(acc.kind)}</span>
                    </span>
                    <Show when={acc.selected}>
                      <Icon name="check" size={15} class="text-a-6" />
                    </Show>
                    <button
                      class="shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-150 text-[12px] text-danger-text px-[6px] py-[3px] rounded-xs hover:bg-danger-soft"
                      title={t("account.removeAccount")}
                      onClick={(e) => removeExisting(acc, e)}
                    >
                      {t("account.remove")}
                    </button>
                  </div>
                )}
              </For>
              <div class="text-[11px] text-dim mt-[2px]">{t("account.orAddNew")}</div>
            </div>
          </Show>

          <button
            class="flex items-center gap-[14px] px-[16px] py-[14px] rounded-ctl glass-card cursor-pointer text-left transition-[background-color,border-color,transform] duration-150 ease-app hover:bg-a-1 hover:border-a-4 hover:-translate-y-px focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5 disabled:opacity-50 disabled:pointer-events-none"
            onClick={startMsa}
            disabled={busy()}
          >
            <Icon name="microsoft" size={24} class="text-a-6" />
            <span class="flex flex-col gap-[2px]">
              <b class="text-[14px] text-fg">{t("account.msaTitle")}</b>
              <small class="text-[12px] text-dim">{t("account.msaDesc")}</small>
            </span>
          </button>
          <button
            class="flex items-center gap-[14px] px-[16px] py-[14px] rounded-ctl glass-card cursor-pointer text-left transition-[background-color,border-color,transform] duration-150 ease-app hover:bg-a-1 hover:border-a-4 hover:-translate-y-px focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
            onClick={() => {
              setStep("offline");
              setError(null);
            }}
          >
            <Icon name="user" size={24} class="text-dim" />
            <span class="flex flex-col gap-[2px]">
              <b class="text-[14px] text-fg">{t("account.offlineTitle")}</b>
              <small class="text-[12px] text-dim">{t("account.offlineDesc")}</small>
            </span>
          </button>
          <button
            class="flex items-center gap-[14px] px-[16px] py-[14px] rounded-ctl glass-card cursor-pointer text-left transition-[background-color,border-color,transform] duration-150 ease-app hover:bg-a-1 hover:border-a-4 hover:-translate-y-px focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-a-5"
            onClick={() => {
              setStep("yggdrasil");
              setError(null);
            }}
          >
            <Icon name="link" size={24} class="text-dim" />
            <span class="flex flex-col gap-[2px]">
              <b class="text-[14px] text-fg">{t("account.yggdrasilTitle")}</b>
              <small class="text-[12px] text-dim">{t("account.yggdrasilDesc")}</small>
            </span>
          </button>
        </div>
      </Show>

      {/* --- 微软设备码 --- */}
      <Show when={step() === "msa"}>
        <div class="p-[18px] flex flex-col gap-[12px]">
          <Show
            when={device()}
            fallback={
              <div class="flex flex-col items-center gap-[10px] p-[16px] text-dim text-[13px]">
                <Spinner />
                <span>{t("account.fetchingCode")}</span>
              </div>
            }
          >
            <p class="m-0 text-[13px] text-fg leading-[1.6]">
              {t("account.msaInstruction")}
            </p>
            <div class="self-center px-[28px] py-[12px] rounded-ctl bg-a-1 text-a-7 font-bold text-[28px] leading-none [font-family:ui-monospace,SFMono-Regular,Menlo,monospace] tracking-[4px] select-all">
              {device()!.user_code}
            </div>
            <p class="m-0 text-[12px] text-dim text-center">
              {t("account.verificationUri")}
              <a
                class="text-a-6 cursor-pointer"
                href={device()!.verification_uri}
                onClick={(e) => {
                  e.preventDefault();
                  shellOpen(device()!.verification_uri);
                }}
              >
                {device()!.verification_uri}
              </a>
            </p>
            <Show when={busy() && !error()}>
              <div class="flex flex-col items-center gap-[10px] px-[16px] pb-[16px] pt-[6px] text-dim text-[13px]">
                <Spinner />
                <span>{t("account.waitingAuth")}</span>
              </div>
            </Show>
          </Show>
        </div>
      </Show>

      {/* --- 离线用户名 --- */}
      <Show when={step() === "offline"}>
        <form class="p-[18px] flex flex-col gap-[12px]" onSubmit={submitOffline}>
          <label for="account-dialog-offline-name" class="sr-only">
            {t("account.offlineNameLabel")}
          </label>
          <input
            id="account-dialog-offline-name"
            name="offlineAccountName"
            class={ACCOUNT_INPUT}
            placeholder={t("account.offlineNamePlaceholder")}
            autocomplete="off"
            spellcheck={false}
            value={offlineName()}
            onInput={(e) => setOfflineName(e.currentTarget.value)}
            maxLength={16}
          />
          <div class="flex justify-end gap-[10px]">
            <button
              type="button"
              class={ACCOUNT_CANCEL_BTN}
              onClick={() => setStep("menu")}
            >
              {t("account.back")}
            </button>
            <button
              type="submit"
              class={ACCOUNT_SUBMIT_BTN}
              disabled={busy() || !offlineName().trim()}
            >
              {busy() ? t("account.adding") : t("account.confirm")}
            </button>
          </div>
        </form>
      </Show>

      {/* --- 外置登录(Yggdrasil) --- */}
      <Show when={step() === "yggdrasil"}>
        <form class="p-[18px] flex flex-col gap-[10px]" onSubmit={submitYggdrasil}>
          <input
            class={ACCOUNT_INPUT}
            placeholder={t("account.yggBasePlaceholder")}
            autocomplete="off"
            spellcheck={false}
            value={ygBase()}
            onInput={(e) => setYgBase(e.currentTarget.value)}
          />
          <input
            class={ACCOUNT_INPUT}
            placeholder={t("account.yggUserPlaceholder")}
            autocomplete="username"
            value={ygUser()}
            onInput={(e) => setYgUser(e.currentTarget.value)}
          />
          <input
            type="password"
            class={ACCOUNT_INPUT}
            placeholder={t("account.yggPassPlaceholder")}
            autocomplete="current-password"
            value={ygPass()}
            onInput={(e) => setYgPass(e.currentTarget.value)}
          />
          <div class="flex justify-end gap-[10px] pt-[2px]">
            <button
              type="button"
              class={ACCOUNT_CANCEL_BTN}
              onClick={() => setStep("menu")}
            >
              {t("account.back")}
            </button>
            <button
              type="submit"
              class={ACCOUNT_SUBMIT_BTN}
              disabled={busy() || !ygBase().trim() || !ygUser().trim()}
            >
              {busy() ? t("account.loggingIn") : t("account.login")}
            </button>
          </div>
        </form>
      </Show>

      <Show when={error()}>
        <div class="mx-[18px] mt-0 mb-[16px] px-[12px] py-[10px] rounded-xs bg-danger-soft text-danger-text text-[12px] leading-[1.6] break-words">
          <Show
            when={/AADSTS700016|client_id|MC_MSA_CLIENT_ID|was not found/i.test(error()!)}
            fallback={error()}
          >
            {t("account.msaClientIdError")}
            <code class="[font-family:ui-monospace,SFMono-Regular,Menlo,monospace] bg-[rgba(192,57,43,0.12)] px-[4px] rounded-[3px]">
              desktop/src-tauri/.env
            </code>
            {t("account.msaClientIdErrorMid")}
            <code class="[font-family:ui-monospace,SFMono-Regular,Menlo,monospace] bg-[rgba(192,57,43,0.12)] px-[4px] rounded-[3px]">
              MC_MSA_CLIENT_ID
            </code>
            {t("account.msaClientIdErrorEnd")}
            <div class="mt-[8px] pt-[8px] border-t border-[rgba(192,57,43,0.25)] text-[11px] opacity-75">
              {error()}
            </div>
          </Show>
        </div>
      </Show>
    </Dialog>
  );
};

export default AccountDialog;
