import { Component, createSignal, Show, onCleanup } from "solid-js";
import { open as shellOpen } from "@tauri-apps/plugin-shell";
import { Spinner, toast } from "../components";
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

  return (
    <div class="pcl-dlg-mask" onClick={props.onClose}>
      <div class="pcl-dlg" onClick={(e) => e.stopPropagation()}>
        <div class="pcl-dlg-head">
          <span>{step() === "offline" ? "离线登录" : step() === "msa" ? "微软登录" : "添加账号"}</span>
          <button class="pcl-dlg-x" onClick={props.onClose} aria-label="关闭">✕</button>
        </div>

        {/* --- 选择登录方式 --- */}
        <Show when={step() === "menu"}>
          <div class="pcl-dlg-body">
            <button class="pcl-dlg-choice pcl-dlg-choice-msa" onClick={startMsa}>
              <span class="pcl-dlg-choice-ico">🪟</span>
              <span class="pcl-dlg-choice-text">
                <b>微软账号</b>
                <small>正版验证,可联机、用正版皮肤</small>
              </span>
            </button>
            <button class="pcl-dlg-choice" onClick={() => { setStep("offline"); setError(null); }}>
              <span class="pcl-dlg-choice-ico">👤</span>
              <span class="pcl-dlg-choice-text">
                <b>离线账号</b>
                <small>仅输入用户名,单机游玩</small>
              </span>
            </button>
          </div>
        </Show>

        {/* --- 微软设备码 --- */}
        <Show when={step() === "msa"}>
          <div class="pcl-dlg-body">
            <Show when={device()} fallback={<div class="pcl-dlg-center"><Spinner /><span>正在获取登录代码…</span></div>}>
              <p class="pcl-dlg-tip">已打开微软登录页并复制代码,在页面输入以下代码完成登录:</p>
              <div class="pcl-dlg-code">{device()!.user_code}</div>
              <p class="pcl-dlg-sub">
                验证地址:<a href={device()!.verification_uri} onClick={(e) => { e.preventDefault(); shellOpen(device()!.verification_uri); }}>{device()!.verification_uri}</a>
              </p>
              <Show when={busy() && !error()}>
                <div class="pcl-dlg-center pcl-dlg-wait"><Spinner /><span>等待你在浏览器中完成授权…</span></div>
              </Show>
            </Show>
          </div>
        </Show>

        {/* --- 离线用户名 --- */}
        <Show when={step() === "offline"}>
          <form class="pcl-dlg-body" onSubmit={submitOffline}>
            <input
              class="pcl-dlg-input"
              placeholder="输入用户名(3-16 位)"
              value={offlineName()}
              onInput={(e) => setOfflineName(e.currentTarget.value)}
              autofocus
              maxLength={16}
            />
            <div class="pcl-dlg-actions">
              <button type="button" class="pcl-dlg-btn" onClick={() => setStep("menu")}>返回</button>
              <button type="submit" class="pcl-dlg-btn primary" disabled={busy() || !offlineName().trim()}>
                {busy() ? "添加中…" : "确定"}
              </button>
            </div>
          </form>
        </Show>

        <Show when={error()}>
          <div class="pcl-dlg-err">
            <Show
              when={/AADSTS700016|client_id|MC_MSA_CLIENT_ID|was not found/i.test(error()!)}
              fallback={error()}
            >
              微软登录需要你自己的 Azure 应用 client_id(默认的老 ID 已被微软拒绝)。
              请到 Azure 注册一个「个人 Microsoft 账户」应用并开启「公共客户端流」,
              把 client_id 写入 <code>desktop/src-tauri/.env</code> 的 <code>MC_MSA_CLIENT_ID</code>,
              重启应用后再试。
              <div class="pcl-dlg-err-raw">{error()}</div>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default PclAccountDialog;
