import { useEffect, useRef, useState } from "react";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner } from "./Spinner";
import { ErrorState } from "./ErrorState";
import { Dialog } from "./Dialog";
import { SkinDialog } from "./SkinDialog";
import { Avatar } from "./Avatar";
import { Icon } from "./Icon";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { useAsync } from "../util/useAsync";
import { api } from "../ipc/api";
import { accountKindLabel } from "../util/accounts";
import { t, useLang } from "../i18n";
import type { AccountSummary, DeviceCode } from "../ipc/types";


// 账号表单输入框(离线用户名 + 外置登录三项)统一样式,避免逐个内联漂移。Blocky:石质暗底深凹倒角。
const ACCOUNT_INPUT =
  "h-[38px] px-[14px] rounded-none text-[13px] text-fg bg-sidebar shadow-input " +
  "placeholder:text-faint transition-[box-shadow] duration-150 ease-app " +
  "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent";

/** 登录弹窗状态机:选择方式 → 微软设备码 / 离线用户名 / 外置登录。 */
type Step = "menu" | "msa" | "offline" | "yggdrasil";

/**
 * AccountDialog —— 主题中性的账号登录弹窗(微软正版 + 离线),两套布局共用。
 *
 * 用桥接令牌(fg / dim / card / 中性 n- / 强调 a-)着色,故在工作台(深色)与经典(浅色)两种布局下
 * 都自动对味。微软走设备码流:start 拿 user_code + 验证地址 → 自动打开浏览器并复制代码 →
 * 后台 poll 阻塞直到用户在网页完成 → 落库并选中 → 回调 onDone。
 */
export function AccountDialog(props: {
  onClose: () => void;
  onDone: (acc: AccountSummary) => void;
}) {
  useLang();
  const [step, setStep] = useState<Step>("menu");
  const [device, setDevice] = useState<DeviceCode | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [offlineName, setOfflineName] = useState("");
  const [ygBase, setYgBase] = useState("");
  const [ygUser, setYgUser] = useState("");
  const [ygPass, setYgPass] = useState("");

  // 已有账号:菜单步顶部列出,可切换/删除(两套布局共用的账号管理入口)。
  const { data: accounts, error: accountsError, refetch: refetchAccounts } = useAsync(
    () => api.listAccounts(),
    [],
  );
  const accountList = accounts ?? [];
  // 正在切换/移除的账号 uuid:列表常驻可见,异步期间禁用该行防重复触发。
  const [pendingAcc, setPendingAcc] = useState<string | null>(null);
  // 打开了皮肤管理弹窗的微软账号(仅微软账号有皮肤 API)。
  const [skinFor, setSkinFor] = useState<AccountSummary | null>(null);

  // 弹窗关闭后别再回调(微软 poll 可能仍在后台轮询)。
  const closed = useRef(false);
  useEffect(() => {
    closed.current = false;
    return () => {
      closed.current = true;
    };
  }, []);

  async function selectExisting(acc: AccountSummary) {
    if (acc.selected) {
      props.onDone(acc);
      return;
    }
    if (pendingAcc) return;
    setPendingAcc(acc.uuid);
    try {
      await api.selectAccount(acc.uuid);
      props.onDone(acc);
    } catch (e) {
      setError(String(e));
    } finally {
      if (!closed.current) setPendingAcc(null);
    }
  }

  async function removeExisting(acc: AccountSummary, e: React.MouseEvent) {
    e.stopPropagation();
    if (pendingAcc) return;
    setPendingAcc(acc.uuid);
    try {
      await api.removeAccount(acc.uuid);
      toast({ type: "success", message: t("account.removed", { name: acc.username }) });
      void refetchAccounts();
    } catch (err) {
      toast({ type: "error", message: t("account.removeFailed", { err: String(err) }) });
    } finally {
      if (!closed.current) setPendingAcc(null);
    }
  }

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
      if (closed.current) return;
      toast({ type: "success", message: t("account.loggedIn", { name: acc.username }) });
      props.onDone(acc);
    } catch (e) {
      if (closed.current) return;
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function submitOffline(e: React.FormEvent) {
    e.preventDefault();
    const name = offlineName.trim();
    if (!name) return;
    setBusy(true);
    setError(null);
    try {
      const acc = await api.addOfflineAccount(name);
      if (closed.current) return;
      toast({ type: "success", message: t("account.offlineAdded", { name: acc.username }) });
      props.onDone(acc);
    } catch (e) {
      if (closed.current) return;
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function submitYggdrasil(e: React.FormEvent) {
    e.preventDefault();
    const base = ygBase.trim();
    const user = ygUser.trim();
    if (!base || !user) return;
    setBusy(true);
    setError(null);
    try {
      const acc = await api.yggdrasilLogin(base, user, ygPass);
      if (closed.current) return;
      toast({ type: "success", message: t("account.loggedInYggdrasil", { name: acc.username }) });
      props.onDone(acc);
    } catch (e) {
      if (closed.current) return;
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const title =
    step === "offline"
      ? t("account.titleOffline")
      : step === "msa"
        ? t("account.titleMsa")
        : step === "yggdrasil"
          ? t("account.titleYggdrasil")
          : t("account.titleAdd");

  return (
    <Dialog
      open
      onClose={props.onClose}
      label={title}
      contentClass="w-[380px] max-w-[calc(100vw-48px)] focus-visible:outline-none"
    >
      <div className="flex items-center justify-between px-[18px] py-[14px] bg-titlebar border-b border-titlebar">
        <Heading size="sub">{title}</Heading>
        <button
          className="border-none bg-transparent text-muted cursor-pointer p-[5px] rounded-none flex items-center transition-colors duration-150 hover:bg-panel-2 hover:text-fg"
          onClick={props.onClose}
          aria-label={t("account.close")}
        >
          <Icon name="close" size={16} />
        </button>
      </div>

      {/* --- 选择登录方式 --- */}
      {step === "menu" && (
        <div className="p-[18px] flex flex-col gap-[12px]">
          {/* 账号列表加载失败:给错误态 + 重试,别让失败看起来像「没有账号」。 */}
          {accountsError != null && (
            <ErrorState compact message={t("account.listLoadFailed")} onRetry={() => void refetchAccounts()} />
          )}
          {/* 已有账号:切换(点击)或移除(✕)。当前账号打勾。 */}
          {accountList.length > 0 && (
            <div className="flex flex-col gap-[6px]">
              {accountList.map((acc) => (
                <div
                  key={acc.uuid}
                  role="button"
                  tabIndex={0}
                  aria-busy={pendingAcc === acc.uuid}
                  className={
                    "group flex items-center gap-[10px] px-[10px] py-[8px] rounded-none bg-panel cursor-pointer transition-[box-shadow,background-color] duration-150 hover:bg-panel-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent aria-[busy=true]:opacity-60 aria-[busy=true]:pointer-events-none " +
                    (acc.selected ? "shadow-raised bg-panel-2" : "shadow-sunken")
                  }
                  onClick={() => selectExisting(acc)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      void selectExisting(acc);
                    }
                  }}
                >
                  <span className="w-[30px] h-[30px] flex-[0_0_30px] rounded-none shadow-input overflow-hidden grid place-items-center text-fg text-[13px] font-semibold bg-sidebar">
                    <Avatar kind={acc.kind} uuid={acc.uuid} />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-[13px] font-semibold text-fg whitespace-nowrap overflow-hidden text-ellipsis">
                      {acc.username}
                    </span>
                    <span className="block text-[11px] text-muted">{accountKindLabel(acc.kind)}</span>
                  </span>
                  {acc.selected && <Icon name="check" size={15} className="text-accent" />}
                  {acc.kind === "microsoft" && (
                    <button
                      className="shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-150 text-[12px] text-accent px-[6px] py-[3px] rounded-none hover:bg-panel-2"
                      title={t("skin.manage")}
                      onClick={(e) => {
                        e.stopPropagation();
                        setSkinFor(acc);
                      }}
                    >
                      {t("skin.manage")}
                    </button>
                  )}
                  <button
                    className="shrink-0 opacity-0 group-hover:opacity-100 transition-opacity duration-150 text-[12px] text-danger-text px-[6px] py-[3px] rounded-none hover:bg-danger-soft"
                    title={t("account.removeAccount")}
                    onClick={(e) => removeExisting(acc, e)}
                  >
                    {t("account.remove")}
                  </button>
                </div>
              ))}
              <div className="text-[11px] text-muted mt-[2px]">{t("account.orAddNew")}</div>
            </div>
          )}

          <button
            className="flex items-center gap-[14px] px-[16px] py-[14px] rounded-none bg-panel-3 shadow-raised cursor-pointer text-left transition-[box-shadow,filter] duration-150 ease-app hover:enabled:brightness-110 active:enabled:shadow-pressed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:opacity-50 disabled:pointer-events-none"
            onClick={startMsa}
            disabled={busy}
          >
            <Icon name="microsoft" size={24} className="text-accent" />
            <span className="flex flex-col gap-[2px]">
              <b className="text-[14px] text-fg">{t("account.msaTitle")}</b>
              <small className="text-[12px] text-muted">{t("account.msaDesc")}</small>
            </span>
          </button>
          <button
            className="flex items-center gap-[14px] px-[16px] py-[14px] rounded-none bg-panel-3 shadow-raised cursor-pointer text-left transition-[box-shadow,filter] duration-150 ease-app hover:brightness-110 active:shadow-pressed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            onClick={() => {
              setStep("offline");
              setError(null);
            }}
          >
            <Icon name="user" size={24} className="text-muted" />
            <span className="flex flex-col gap-[2px]">
              <b className="text-[14px] text-fg">{t("account.offlineTitle")}</b>
              <small className="text-[12px] text-muted">{t("account.offlineDesc")}</small>
            </span>
          </button>
          <button
            className="flex items-center gap-[14px] px-[16px] py-[14px] rounded-none bg-panel-3 shadow-raised cursor-pointer text-left transition-[box-shadow,filter] duration-150 ease-app hover:brightness-110 active:shadow-pressed focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            onClick={() => {
              setStep("yggdrasil");
              setError(null);
            }}
          >
            <Icon name="link" size={24} className="text-muted" />
            <span className="flex flex-col gap-[2px]">
              <b className="text-[14px] text-fg">{t("account.yggdrasilTitle")}</b>
              <small className="text-[12px] text-muted">{t("account.yggdrasilDesc")}</small>
            </span>
          </button>
        </div>
      )}

      {/* --- 微软设备码 --- */}
      {step === "msa" && (
        <div className="p-[18px] flex flex-col gap-[12px]">
          {device ? (
            <>
              <p className="m-0 text-[13px] text-fg leading-[1.6]">{t("account.msaInstruction")}</p>
              <div className="self-center px-[28px] py-[12px] rounded-none bg-sidebar shadow-input text-accent font-pixel text-[24px] leading-none tracking-[4px] select-all">
                {device.user_code}
              </div>
              <p className="m-0 text-[12px] text-muted text-center">
                {t("account.verificationUri")}
                <a
                  className="text-accent cursor-pointer"
                  href={device.verification_uri}
                  onClick={(e) => {
                    e.preventDefault();
                    shellOpen(device.verification_uri);
                  }}
                >
                  {device.verification_uri}
                </a>
              </p>
              {busy && !error && (
                <div className="flex flex-col items-center gap-[10px] px-[16px] pb-[16px] pt-[6px] text-muted text-[13px]">
                  <Spinner />
                  <span>{t("account.waitingAuth")}</span>
                </div>
              )}
            </>
          ) : (
            <div className="flex flex-col items-center gap-[10px] p-[16px] text-muted text-[13px]">
              <Spinner />
              <span>{t("account.fetchingCode")}</span>
            </div>
          )}
        </div>
      )}

      {/* --- 离线用户名 --- */}
      {step === "offline" && (
        <form className="p-[18px] flex flex-col gap-[12px]" onSubmit={submitOffline}>
          <label htmlFor="account-dialog-offline-name" className="sr-only">
            {t("account.offlineNameLabel")}
          </label>
          <input
            id="account-dialog-offline-name"
            name="offlineAccountName"
            className={ACCOUNT_INPUT}
            placeholder={t("account.offlineNamePlaceholder")}
            autoComplete="off"
            spellCheck={false}
            value={offlineName}
            onChange={(e) => setOfflineName(e.currentTarget.value)}
            maxLength={16}
          />
          <div className="flex justify-end gap-[10px]">
            <Button type="button" variant="ghost" onClick={() => setStep("menu")}>
              {t("account.back")}
            </Button>
            <Button type="submit" variant="primary" disabled={busy || !offlineName.trim()}>
              {busy ? t("account.adding") : t("account.confirm")}
            </Button>
          </div>
        </form>
      )}

      {/* --- 外置登录(Yggdrasil) --- */}
      {step === "yggdrasil" && (
        <form className="p-[18px] flex flex-col gap-[10px]" onSubmit={submitYggdrasil}>
          <input
            className={ACCOUNT_INPUT}
            placeholder={t("account.yggBasePlaceholder")}
            autoComplete="off"
            spellCheck={false}
            value={ygBase}
            onChange={(e) => setYgBase(e.currentTarget.value)}
          />
          <input
            className={ACCOUNT_INPUT}
            placeholder={t("account.yggUserPlaceholder")}
            autoComplete="username"
            value={ygUser}
            onChange={(e) => setYgUser(e.currentTarget.value)}
          />
          <input
            type="password"
            className={ACCOUNT_INPUT}
            placeholder={t("account.yggPassPlaceholder")}
            autoComplete="current-password"
            value={ygPass}
            onChange={(e) => setYgPass(e.currentTarget.value)}
          />
          <div className="flex justify-end gap-[10px] pt-[2px]">
            <Button type="button" variant="ghost" onClick={() => setStep("menu")}>
              {t("account.back")}
            </Button>
            <Button type="submit" variant="primary" disabled={busy || !ygBase.trim() || !ygUser.trim()}>
              {busy ? t("account.loggingIn") : t("account.login")}
            </Button>
          </div>
        </form>
      )}

      {error && (
        <div className="mx-[18px] mt-0 mb-[16px] px-[12px] py-[10px] rounded-none bg-danger-soft shadow-input text-danger-text text-[12px] leading-[1.6] break-words">
          {/AADSTS700016|client_id|MC_MSA_CLIENT_ID|was not found/i.test(error) ? (
            <>
              {t("account.msaClientIdError")}
              <code className="[font-family:ui-monospace,SFMono-Regular,Menlo,monospace] bg-window/40 px-[4px] rounded-none">
                desktop/src-tauri/.env
              </code>
              {t("account.msaClientIdErrorMid")}
              <code className="[font-family:ui-monospace,SFMono-Regular,Menlo,monospace] bg-window/40 px-[4px] rounded-none">
                MC_MSA_CLIENT_ID
              </code>
              {t("account.msaClientIdErrorEnd")}
              <div className="mt-[8px] pt-[8px] border-t border-titlebar text-[11px] opacity-75">{error}</div>
            </>
          ) : (
            error
          )}
        </div>
      )}

      {skinFor && (
        <SkinDialog uuid={skinFor.uuid} username={skinFor.username} onClose={() => setSkinFor(null)} />
      )}
    </Dialog>
  );
}

export default AccountDialog;
