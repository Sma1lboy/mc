import { Component, createEffect, createResource, createSignal, For, Show, onCleanup } from "solid-js";
import { open as openFile } from "@tauri-apps/plugin-dialog";
import { Spinner } from "./Spinner";
import { ErrorState } from "./ErrorState";
import { Dialog } from "./Dialog";
import { Icon } from "./Icon";
import { Button } from "./Button";
import { Heading } from "./Typography";
import { toast } from "./Toast";
import { api } from "../ipc/api";
import { cached, invalidate } from "../ipc/cache";
import { t } from "../i18n";
import type { ProfileSkins, CapeInfo } from "../ipc/types";

/**
 * SkinDialog —— 微软账号的皮肤 / 披风管理弹窗。
 *
 * 读取 profile 端点(用账号的 Minecraft access token):展示当前皮肤预览 + 模型变体、
 * 披风列表(高亮使用中、点击切换/隐藏),并提供「上传皮肤」(本地 PNG → classic/slim)。
 * 仅微软正版账号有此 API,离线 / 外置账号不应打开本弹窗。
 */
export const SkinDialog: Component<{
  uuid: string;
  username: string;
  onClose: () => void;
}> = (props) => {
  const [profile, { mutate, refetch }] = createResource<ProfileSkins>(() =>
    cached(`skinProfile|${props.uuid}`, () => api.skinProfile(props.uuid)),
  );
  const [busy, setBusy] = createSignal(false);
  const [variant, setVariant] = createSignal<"classic" | "slim">("classic");

  let closed = false;
  onCleanup(() => {
    closed = true;
  });

  // 当前生效的皮肤 / 披风。
  const activeSkin = () => {
    const p = profile();
    if (!p) return undefined;
    return p.skins?.find((s) => s.state === "ACTIVE") ?? p.skins?.[0];
  };
  const activeCapeId = () => profile()?.capes?.find((c) => c.state === "ACTIVE")?.id ?? null;

  // 加载到 profile 后把上传默认变体同步成当前皮肤的变体。
  createEffect(() => {
    const v = profile()?.skins?.find((s) => s.state === "ACTIVE")?.variant;
    if (v === "slim" || v === "classic") setVariant(v);
  });

  async function pickAndUpload() {
    if (busy()) return;
    let path: string | null;
    try {
      const sel = await openFile({
        multiple: false,
        directory: false,
        filters: [{ name: "PNG", extensions: ["png"] }],
      });
      path = Array.isArray(sel) ? (sel[0] ?? null) : sel;
    } catch {
      return;
    }
    if (!path) return;
    setBusy(true);
    try {
      const updated = await api.skinUpload(props.uuid, path, variant());
      if (closed) return;
      mutate(updated);
      invalidate(`skinProfile|${props.uuid}`); // 皮肤已变,丢弃旧缓存,重开取最新
      toast({ type: "success", message: t("skin.uploaded") });
    } catch (e) {
      if (closed) return;
      toast({ type: "error", message: t("skin.uploadFailed", { err: String(e) }) });
    } finally {
      if (!closed) setBusy(false);
    }
  }

  async function chooseCape(capeId: string | null) {
    if (busy()) return;
    if (capeId === activeCapeId()) return;
    setBusy(true);
    try {
      const updated = await api.skinSetCape(props.uuid, capeId);
      if (closed) return;
      mutate(updated);
      invalidate(`skinProfile|${props.uuid}`); // 披风已变,丢弃旧缓存,重开取最新
      toast({ type: "success", message: t("skin.capeUpdated") });
    } catch (e) {
      if (closed) return;
      toast({ type: "error", message: t("skin.capeFailed", { err: String(e) }) });
    } finally {
      if (!closed) setBusy(false);
    }
  }

  const variantLabel = (v: string | undefined) =>
    v === "slim" ? t("skin.variantSlim") : t("skin.variantClassic");

  return (
    <Dialog
      open
      onClose={props.onClose}
      label={t("skin.title")}
      contentClass="w-[460px] max-w-[calc(100vw-48px)] focus-visible:outline-none"
    >
      <div class="flex items-center justify-between px-[18px] py-[14px] bg-titlebar border-b border-titlebar">
        <Heading size="sub">{t("skin.title")}</Heading>
        <button
          class="border-none bg-transparent text-muted cursor-pointer p-[5px] rounded-none flex items-center transition-colors duration-150 hover:bg-panel-2 hover:text-fg"
          onClick={props.onClose}
          aria-label={t("skin.close")}
        >
          <Icon name="close" size={16} />
        </button>
      </div>

      <Show
        when={!profile.loading}
        fallback={
          <div class="flex flex-col items-center gap-[10px] p-[40px] text-muted text-[13px]">
            <Spinner />
            <span>{t("skin.loading")}</span>
          </div>
        }
      >
        <Show
          when={!profile.error}
          fallback={
            <div class="p-[18px]">
              <ErrorState
                compact
                message={t("skin.loadFailed", { err: String(profile.error) })}
                onRetry={() => void refetch()}
              />
            </div>
          }
        >
          <div class="p-[18px] flex flex-col gap-[18px]">
            {/* 当前皮肤预览 + 变体 + 上传 */}
            <div class="flex gap-[16px]">
              <div class="w-[96px] h-[128px] shrink-0 grid place-items-center bg-sidebar shadow-input overflow-hidden">
                <Show
                  when={activeSkin()?.url}
                  fallback={<Icon name="user" size={36} class="text-faint" />}
                >
                  <img
                    src={activeSkin()!.url}
                    alt={props.username}
                    class="w-full h-full object-contain [image-rendering:pixelated]"
                  />
                </Show>
              </div>
              <div class="min-w-0 flex-1 flex flex-col gap-[10px]">
                <div>
                  <div class="text-[14px] font-semibold text-fg truncate">{props.username}</div>
                  <div class="text-[12px] text-muted mt-[2px]">
                    {t("skin.variant")}:{variantLabel(activeSkin()?.variant)}
                  </div>
                </div>
                <div class="flex flex-col gap-[6px]">
                  <div class="text-[11px] text-muted">{t("skin.chooseVariant")}</div>
                  <div class="flex gap-[8px]">
                    <For each={["classic", "slim"] as const}>
                      {(v) => (
                        <button
                          class="px-[12px] py-[6px] text-[12px] rounded-none shadow-input bg-sidebar text-fg cursor-pointer transition-[box-shadow,background-color] duration-150 hover:bg-panel-2 aria-[pressed=true]:shadow-pressed aria-[pressed=true]:bg-panel-2 aria-[pressed=true]:text-accent"
                          aria-pressed={variant() === v}
                          onClick={() => setVariant(v)}
                          disabled={busy()}
                        >
                          {variantLabel(v)}
                        </button>
                      )}
                    </For>
                  </div>
                </div>
                <Button variant="primary" onClick={pickAndUpload} disabled={busy()}>
                  {busy() ? t("skin.uploading") : t("skin.upload")}
                </Button>
              </div>
            </div>

            {/* 披风列表 */}
            <div class="flex flex-col gap-[8px]">
              <div class="text-[13px] font-semibold text-fg">{t("skin.capes")}</div>
              <Show
                when={(profile()?.capes?.length ?? 0) > 0}
                fallback={<div class="text-[12px] text-muted">{t("skin.noCapes")}</div>}
              >
                <div class="grid grid-cols-3 gap-[10px]">
                  {/* 不戴披风 */}
                  <CapeTile
                    label={t("skin.capeNone")}
                    active={activeCapeId() === null}
                    disabled={busy()}
                    onClick={() => void chooseCape(null)}
                  />
                  <For each={profile()!.capes!}>
                    {(cape: CapeInfo) => (
                      <CapeTile
                        label={cape.alias || t("skin.capes")}
                        url={cape.url}
                        active={cape.state === "ACTIVE"}
                        activeText={t("skin.capeActive")}
                        disabled={busy()}
                        onClick={() => void chooseCape(cape.id)}
                      />
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </div>
        </Show>
      </Show>
    </Dialog>
  );
};

const CapeTile: Component<{
  label: string;
  url?: string;
  active: boolean;
  activeText?: string;
  disabled?: boolean;
  onClick: () => void;
}> = (props) => (
  <button
    class="flex flex-col items-center gap-[6px] p-[8px] rounded-none bg-panel cursor-pointer transition-[box-shadow,background-color] duration-150 hover:bg-panel-2 aria-[pressed=true]:shadow-raised aria-[pressed=true]:bg-panel-2 disabled:opacity-50 disabled:pointer-events-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
    aria-pressed={props.active}
    onClick={props.onClick}
    disabled={props.disabled}
  >
    <span class="w-[48px] h-[64px] grid place-items-center bg-sidebar shadow-input overflow-hidden">
      <Show when={props.url} fallback={<Icon name="close" size={18} class="text-faint" />}>
        <img
          src={props.url}
          alt={props.label}
          class="w-full h-full object-contain [image-rendering:pixelated]"
        />
      </Show>
    </span>
    <span class="text-[11px] text-fg text-center truncate max-w-full leading-tight">{props.label}</span>
    <Show when={props.active}>
      <span class="text-[10px] text-accent leading-none">{props.activeText ?? "✓"}</span>
    </Show>
  </button>
);

export default SkinDialog;
