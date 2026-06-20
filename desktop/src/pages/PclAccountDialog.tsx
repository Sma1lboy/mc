import { Component, createSignal, Show, onCleanup } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner, Dialog, toast } from "../components";
import { api } from "../ipc/api";
import type { AccountSummary, DeviceCode } from "../ipc/types";
import "./PclAccountDialog.css";

/** 登录弹窗状态机:选择方式 → 微软设备码 / 离线用户名。 */
type Step = "menu" | "msa" | "offline";

/**
 * PclAccountDialog —— 账号登录弹窗(微软正版 + 离线)。
 *
 * 微软走设备码流:start 拿 user_code + 验证地址 → 自动打开浏览器并复制代码 →
 * 后台 poll 阻塞直到用户在网页完成 → 落库并选中 → 回调 onDone。
 */
const PclAccountDialog: Component<{
  onClose: () => void;
  onDone: (acc: AccountSummary) => void;
}> = (props) => {
  const [step, setStep] = createSignal<Step>("menu");
  const [device, setDevice] = createSignal<DeviceCode | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [offlineName, setOfflineName] = createSignal("");

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
      toast({ type: "success", message: `已登录:${acc.username}` });
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
      toast({ type: "success", message: `已添加离线账号:${acc.username}` });
      props.onDone(acc);
    } catch (e) {
      if (closed) return;
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const title = () =>
    step() === "offline" ? "离线登录" : step() === "msa" ? "微软登录" : "添加账号";

  return (
    // Ark Dialog 负责焦点陷阱 / Esc / 点遮罩关闭 / 滚动锁;遮罩复用 pcl-dlg-mask
    // 的亚克力模糊 + [data-blur=off] / prefers-reduced-transparency 逃生口。
    <Dialog
      open
      onClose={props.onClose}
      label={title()}
      backdropClass="pcl-dlg-mask"
      contentClass="w-[380px] max-w-[calc(100vw-48px)] bg-pcl-card rounded-[8px] shadow-[0_12px_40px_rgba(52,61,74,0.35)] overflow-hidden focus:outline-none"
    >
        <div class="flex items-center justify-between px-[18px] py-[14px] text-[15px] font-bold text-white bg-[linear-gradient(90deg,var(--pcl-blue-hover),var(--pcl-blue))]">
          <span>{title()}</span>
          <button
            class="border-none bg-transparent text-white text-[15px] cursor-pointer opacity-85 px-[6px] py-[2px] rounded-[4px] hover:bg-white/20 hover:opacity-100"
            onClick={props.onClose}
            aria-label="关闭"
          >
            ✕
          </button>
        </div>

        {/* --- 选择登录方式 --- */}
        <Show when={step() === "menu"}>
          <div class="p-[18px] flex flex-col gap-[12px]">
            <button
              class="flex items-center gap-[14px] px-[16px] py-[14px] border border-pcl-line rounded-[6px] bg-pcl-card cursor-pointer text-left [transition:background_0.15s_ease,border-color_0.15s_ease,transform_0.1s_ease] hover:bg-pcl-blue-lightest hover:border-pcl-blue hover:-translate-y-px"
              onClick={startMsa}
            >
              <span class="text-[26px]">🪟</span>
              <span class="flex flex-col gap-[2px]">
                <b class="text-[14px] text-pcl-text">微软账号</b>
                <small class="text-[12px] text-pcl-text3">正版验证,可联机、用正版皮肤</small>
              </span>
            </button>
            <button
              class="flex items-center gap-[14px] px-[16px] py-[14px] border border-pcl-line rounded-[6px] bg-pcl-card cursor-pointer text-left [transition:background_0.15s_ease,border-color_0.15s_ease,transform_0.1s_ease] hover:bg-pcl-blue-lightest hover:border-pcl-blue-soft hover:-translate-y-px"
              onClick={() => { setStep("offline"); setError(null); }}
            >
              <span class="text-[26px]">👤</span>
              <span class="flex flex-col gap-[2px]">
                <b class="text-[14px] text-pcl-text">离线账号</b>
                <small class="text-[12px] text-pcl-text3">仅输入用户名,单机游玩</small>
              </span>
            </button>
          </div>
        </Show>

        {/* --- 微软设备码 --- */}
        <Show when={step() === "msa"}>
          <div class="p-[18px] flex flex-col gap-[12px]">
            <Show when={device()} fallback={<div class="flex flex-col items-center gap-[10px] p-[16px] text-pcl-text3 text-[13px]"><Spinner /><span>正在获取登录代码…</span></div>}>
              <p class="m-0 text-[13px] text-pcl-text2 leading-[1.6]">已打开微软登录页并复制代码,在页面输入以下代码完成登录:</p>
              <div class="self-center px-[28px] py-[12px] rounded-[6px] bg-pcl-blue-bg text-pcl-blue-dark font-bold text-[28px] leading-none [font-family:ui-monospace,SFMono-Regular,Menlo,monospace] tracking-[4px] select-all">{device()!.user_code}</div>
              <p class="m-0 text-[12px] text-pcl-text3 text-center">
                验证地址:<a class="text-pcl-blue cursor-pointer" href={device()!.verification_uri} onClick={(e) => { e.preventDefault(); shellOpen(device()!.verification_uri); }}>{device()!.verification_uri}</a>
              </p>
              <Show when={busy() && !error()}>
                <div class="flex flex-col items-center gap-[10px] px-[16px] pb-[16px] pt-[6px] text-pcl-text3 text-[13px]"><Spinner /><span>等待你在浏览器中完成授权…</span></div>
              </Show>
            </Show>
          </div>
        </Show>

        {/* --- 离线用户名 --- */}
        <Show when={step() === "offline"}>
          <form class="p-[18px] flex flex-col gap-[12px]" onSubmit={submitOffline}>
            <input
              class="h-[40px] px-[14px] border border-pcl-line rounded-[5px] text-[14px] text-pcl-text bg-pcl-gray-bg outline-none transition-[border-color,background-color] duration-150 ease-app focus:border-pcl-blue focus:bg-pcl-card"
              placeholder="输入用户名(3-16 位)"
              value={offlineName()}
              onInput={(e) => setOfflineName(e.currentTarget.value)}
              autofocus
              maxLength={16}
            />
            <div class="flex justify-end gap-[10px]">
              <button
                type="button"
                class="h-[36px] px-[18px] border border-pcl-line rounded-[4px] bg-pcl-card text-pcl-text text-[13px] cursor-pointer transition-[background-color,border-color] duration-150 ease-app hover:bg-pcl-blue-lightest hover:border-pcl-blue-soft"
                onClick={() => setStep("menu")}
              >
                返回
              </button>
              <button
                type="submit"
                class="h-[36px] px-[18px] rounded-[4px] bg-pcl-blue text-white border border-pcl-blue text-[13px] cursor-pointer transition-[background-color,border-color] duration-150 ease-app hover:not-disabled:bg-pcl-blue-hover disabled:opacity-50 disabled:cursor-not-allowed"
                disabled={busy() || !offlineName().trim()}
              >
                {busy() ? "添加中…" : "确定"}
              </button>
            </div>
          </form>
        </Show>

        <Show when={error()}>
          <div class="mx-[18px] mt-0 mb-[16px] px-[12px] py-[10px] rounded-[5px] bg-[#fdecec] text-[#c0392b] text-[12px] leading-[1.6] break-words">
            <Show
              when={/AADSTS700016|client_id|MC_MSA_CLIENT_ID|was not found/i.test(error()!)}
              fallback={error()}
            >
              微软登录需要你自己的 Azure 应用 client_id(默认的老 ID 已被微软拒绝)。
              请到 Azure 注册一个「个人 Microsoft 账户」应用并开启「公共客户端流」,
              把 client_id 写入 <code class="[font-family:ui-monospace,SFMono-Regular,Menlo,monospace] bg-[rgba(192,57,43,0.12)] px-[4px] rounded-[3px]">desktop/src-tauri/.env</code> 的 <code class="[font-family:ui-monospace,SFMono-Regular,Menlo,monospace] bg-[rgba(192,57,43,0.12)] px-[4px] rounded-[3px]">MC_MSA_CLIENT_ID</code>,
              重启应用后再试。
              <div class="mt-[8px] pt-[8px] border-t border-[rgba(192,57,43,0.25)] text-[11px] opacity-75">{error()}</div>
            </Show>
          </div>
        </Show>
    </Dialog>
  );
};

export default PclAccountDialog;
